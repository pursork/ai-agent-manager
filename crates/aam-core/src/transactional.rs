use std::error::Error;
use std::fmt;

/// A single operation that mutates persistent local state and must never be
/// allowed to leave that state half-changed.
///
/// Implementations correspond to the "snapshot → apply → verify → rollback"
/// standard operation sequence from `docs/03-credential-account-module.md`
/// §3.5 (its step 1, "confirm the target tool isn't currently running", is
/// backend-specific pre-flight and is left to callers, not this trait).
///
/// Every `aam-switcher` / `aam-sync` / `aam-memory` operation that touches
/// local persistent state (account switch, vault write, index update)
/// should implement this rather than hand-rolling its own snapshot/rollback
/// logic, per `docs/02-architecture.md` §2.6.
pub trait TransactionalOp {
    /// Enough state, captured before `apply`, to undo it later.
    type Snapshot;
    /// Error type shared by every step.
    type Error;

    /// Record whatever is needed to restore the pre-`apply` state.
    /// Must not have any side effects on its own.
    fn snapshot(&self) -> Result<Self::Snapshot, Self::Error>;

    /// Perform the actual mutation.
    fn apply(&mut self) -> Result<(), Self::Error>;

    /// Confirm the mutation actually took effect and is usable — not just
    /// that a write syscall returned `Ok`. For an account switch this means
    /// a real liveness check, not "the file parsed".
    fn verify(&self) -> Result<(), Self::Error>;

    /// Undo `apply` using a previously captured snapshot.
    fn rollback(&mut self, snapshot: Self::Snapshot) -> Result<(), Self::Error>;
}

/// Error returned when `apply`/`verify` failed and the subsequent rollback
/// attempt (either `rollback` itself, or the confirmation `verify` run
/// after it) also failed. This is the one outcome callers must treat as
/// "state is now unknown" rather than "safely back to where we started".
#[derive(Debug)]
pub struct RollbackFailed<E> {
    /// The error that triggered the rollback attempt (from `apply` or `verify`).
    pub original: E,
    /// The error from the rollback attempt itself, or from the post-rollback
    /// `verify` call if `rollback()` returned `Ok` but the restored state
    /// still didn't verify.
    pub rollback_error: E,
}

impl<E: fmt::Display> fmt::Display for RollbackFailed<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "operation failed ({}), and rollback also failed ({}); state is now unknown",
            self.original, self.rollback_error
        )
    }
}

impl<E: fmt::Debug + fmt::Display> Error for RollbackFailed<E> {}

/// The outcome of a failed [`execute`] call.
#[derive(Debug)]
pub enum ExecuteError<E> {
    /// `snapshot` itself failed; `apply` was never called, nothing changed.
    SnapshotFailed(E),
    /// `apply` or `verify` failed, but rollback succeeded and a post-rollback
    /// `verify` confirmed the pre-`apply` state is usable again.
    RolledBack(E),
    /// `apply` or `verify` failed, and rollback did not cleanly restore a
    /// verified state. See [`RollbackFailed`].
    RollbackFailed(RollbackFailed<E>),
}

impl<E: fmt::Display> fmt::Display for ExecuteError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExecuteError::SnapshotFailed(e) => write!(f, "snapshot failed: {e}"),
            ExecuteError::RolledBack(e) => write!(f, "{e} (rolled back successfully)"),
            ExecuteError::RollbackFailed(e) => write!(f, "{e}"),
        }
    }
}

impl<E: fmt::Debug + fmt::Display> Error for ExecuteError<E> {}

/// Runs `op` through snapshot → apply → verify, rolling back automatically
/// (and re-verifying the rollback) if `apply` or `verify` fails.
///
/// This never leaves an intermediate state silently visible: on any
/// failure it always attempts a rollback before returning, and never
/// swallows the original error — see `docs/02-architecture.md` §2.6.
pub fn execute<T: TransactionalOp>(op: &mut T) -> Result<(), ExecuteError<T::Error>> {
    let snapshot = op.snapshot().map_err(ExecuteError::SnapshotFailed)?;

    if let Err(apply_err) = op.apply() {
        return Err(roll_back_after(op, snapshot, apply_err));
    }

    if let Err(verify_err) = op.verify() {
        return Err(roll_back_after(op, snapshot, verify_err));
    }

    Ok(())
}

/// Attempts to undo `op` back to `snapshot` after it failed with
/// `original`, re-verifying the restored state before declaring the
/// rollback clean.
fn roll_back_after<T: TransactionalOp>(
    op: &mut T,
    snapshot: T::Snapshot,
    original: T::Error,
) -> ExecuteError<T::Error> {
    if let Err(rollback_error) = op.rollback(snapshot) {
        return ExecuteError::RollbackFailed(RollbackFailed {
            original,
            rollback_error,
        });
    }

    if let Err(verify_error) = op.verify() {
        return ExecuteError::RollbackFailed(RollbackFailed {
            original,
            rollback_error: verify_error,
        });
    }

    ExecuteError::RolledBack(original)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    struct TestError(&'static str);

    impl fmt::Display for TestError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.0)
        }
    }
    impl Error for TestError {}

    /// A fake op whose every step can be told to succeed or fail, recording
    /// which steps ran so tests can assert ordering as well as outcome.
    struct FakeOp {
        fail_apply: bool,
        fail_verify: bool,
        fail_rollback: bool,
        fail_post_rollback_verify: bool,
        log: Vec<&'static str>,
        rolled_back: bool,
    }

    impl FakeOp {
        fn new() -> Self {
            Self {
                fail_apply: false,
                fail_verify: false,
                fail_rollback: false,
                fail_post_rollback_verify: false,
                log: Vec::new(),
                rolled_back: false,
            }
        }
    }

    impl TransactionalOp for FakeOp {
        type Snapshot = ();
        type Error = TestError;

        fn snapshot(&self) -> Result<(), TestError> {
            Ok(())
        }

        fn apply(&mut self) -> Result<(), TestError> {
            self.log.push("apply");
            if self.fail_apply {
                return Err(TestError("apply failed"));
            }
            Ok(())
        }

        fn verify(&self) -> Result<(), TestError> {
            if self.rolled_back {
                return if self.fail_post_rollback_verify {
                    Err(TestError("post-rollback verify failed"))
                } else {
                    Ok(())
                };
            }
            if self.fail_verify {
                return Err(TestError("verify failed"));
            }
            Ok(())
        }

        fn rollback(&mut self, _snapshot: ()) -> Result<(), TestError> {
            self.log.push("rollback");
            if self.fail_rollback {
                return Err(TestError("rollback failed"));
            }
            self.rolled_back = true;
            Ok(())
        }
    }

    #[test]
    fn success_path_never_rolls_back() {
        let mut op = FakeOp::new();
        let result = execute(&mut op);
        assert!(result.is_ok());
        assert_eq!(op.log, vec!["apply"]);
    }

    #[test]
    fn apply_failure_triggers_rollback_and_is_reported() {
        let mut op = FakeOp::new();
        op.fail_apply = true;
        let result = execute(&mut op);
        match result {
            Err(ExecuteError::RolledBack(e)) => assert_eq!(e, TestError("apply failed")),
            other => panic!("expected RolledBack, got {other:?}"),
        }
        assert_eq!(op.log, vec!["apply", "rollback"]);
    }

    #[test]
    fn verify_failure_triggers_rollback_and_is_reported() {
        let mut op = FakeOp::new();
        op.fail_verify = true;
        let result = execute(&mut op);
        match result {
            Err(ExecuteError::RolledBack(e)) => assert_eq!(e, TestError("verify failed")),
            other => panic!("expected RolledBack, got {other:?}"),
        }
        assert_eq!(op.log, vec!["apply", "rollback"]);
    }

    #[test]
    fn rollback_failure_is_reported_distinctly() {
        let mut op = FakeOp::new();
        op.fail_apply = true;
        op.fail_rollback = true;
        let result = execute(&mut op);
        match result {
            Err(ExecuteError::RollbackFailed(RollbackFailed {
                original,
                rollback_error,
            })) => {
                assert_eq!(original, TestError("apply failed"));
                assert_eq!(rollback_error, TestError("rollback failed"));
            }
            other => panic!("expected RollbackFailed, got {other:?}"),
        }
    }

    #[test]
    fn post_rollback_verify_failure_is_reported_distinctly() {
        let mut op = FakeOp::new();
        op.fail_apply = true;
        op.fail_post_rollback_verify = true;
        let result = execute(&mut op);
        match result {
            Err(ExecuteError::RollbackFailed(RollbackFailed {
                original,
                rollback_error,
            })) => {
                assert_eq!(original, TestError("apply failed"));
                assert_eq!(rollback_error, TestError("post-rollback verify failed"));
            }
            other => panic!("expected RollbackFailed, got {other:?}"),
        }
    }

    #[test]
    fn snapshot_failure_never_calls_apply_or_rollback() {
        struct FailSnapshot;
        impl TransactionalOp for FailSnapshot {
            type Snapshot = ();
            type Error = TestError;
            fn snapshot(&self) -> Result<(), TestError> {
                Err(TestError("snapshot failed"))
            }
            fn apply(&mut self) -> Result<(), TestError> {
                panic!("apply must not run if snapshot failed");
            }
            fn verify(&self) -> Result<(), TestError> {
                panic!("verify must not run if snapshot failed");
            }
            fn rollback(&mut self, _snapshot: ()) -> Result<(), TestError> {
                panic!("rollback must not run if snapshot failed");
            }
        }

        let mut op = FailSnapshot;
        let result = execute(&mut op);
        match result {
            Err(ExecuteError::SnapshotFailed(e)) => assert_eq!(e, TestError("snapshot failed")),
            other => panic!("expected SnapshotFailed, got {other:?}"),
        }
    }
}
