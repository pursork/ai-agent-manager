//! Provider management screen: register/list third-party endpoints
//! (`docs/03-credential-account-module.md` §3.1), the graphical
//! equivalent of `aam provider add/list`
//! (`crates/aam-cli/src/commands.rs::run_provider`). Never displays a
//! saved API key -- same rule the CLI's `provider list` follows.

use aam_switcher::{ProviderKind, ProviderRecord, ProviderRegistry};
use iced::widget::{button, checkbox, column, container, row, text, text_input};
use iced::{Element, Length, Task};

use crate::task::perform;

pub struct NewProviderForm {
    pub kind: ProviderKind,
    pub id: String,
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub supports_websockets: bool,
    pub reasoning_effort: String,
    pub plan_reasoning_effort: String,
}

impl Default for NewProviderForm {
    fn default() -> Self {
        Self {
            kind: ProviderKind::Cpa,
            id: String::new(),
            base_url: String::new(),
            model: String::new(),
            api_key: String::new(),
            supports_websockets: false,
            reasoning_effort: "high".to_string(),
            plan_reasoning_effort: "high".to_string(),
        }
    }
}

#[derive(Default)]
pub struct State {
    pub providers: Vec<ProviderRecord>,
    pub form: NewProviderForm,
    pub saving: bool,
    pub status_message: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Loaded(Result<Vec<ProviderRecord>, String>),
    KindChanged(ProviderKind),
    IdChanged(String),
    BaseUrlChanged(String),
    ModelChanged(String),
    ApiKeyChanged(String),
    SupportsWebsocketsToggled(bool),
    ReasoningEffortChanged(String),
    PlanReasoningEffortChanged(String),
    Submit,
    Saved(Result<String, String>),
}

pub fn load() -> Task<Message> {
    perform(|| ProviderRegistry::open_default().list().map_err(|e| e.to_string()), Message::Loaded)
}

pub fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::Loaded(Ok(providers)) => {
            state.providers = providers;
            Task::none()
        }
        Message::Loaded(Err(e)) => {
            state.status_message = Some(format!("加载 Provider 列表失败: {e}"));
            Task::none()
        }
        Message::KindChanged(kind) => {
            state.form.kind = kind;
            Task::none()
        }
        Message::IdChanged(v) => {
            state.form.id = v;
            Task::none()
        }
        Message::BaseUrlChanged(v) => {
            state.form.base_url = v;
            Task::none()
        }
        Message::ModelChanged(v) => {
            state.form.model = v;
            Task::none()
        }
        Message::ApiKeyChanged(v) => {
            state.form.api_key = v;
            Task::none()
        }
        Message::SupportsWebsocketsToggled(v) => {
            state.form.supports_websockets = v;
            Task::none()
        }
        Message::ReasoningEffortChanged(v) => {
            state.form.reasoning_effort = v;
            Task::none()
        }
        Message::PlanReasoningEffortChanged(v) => {
            state.form.plan_reasoning_effort = v;
            Task::none()
        }
        Message::Submit => {
            if state.form.kind == ProviderKind::Cpa && state.form.model.trim().is_empty() {
                state.status_message = Some("kind=cpa 时 model 不能为空".to_string());
                return Task::none();
            }
            if state.form.api_key.trim().is_empty() {
                state.status_message = Some("API key 不能为空".to_string());
                return Task::none();
            }
            state.saving = true;
            let id = if state.form.id.trim().is_empty() {
                state.form.kind.to_string()
            } else {
                state.form.id.clone()
            };
            let model = match state.form.kind {
                ProviderKind::DeepseekV4Flash => "deepseek-v4-flash".to_string(),
                ProviderKind::Cpa => state.form.model.clone(),
            };
            let record = ProviderRecord {
                id: id.clone(),
                kind: state.form.kind,
                base_url: state.form.base_url.clone(),
                model,
                reasoning_effort: state.form.reasoning_effort.clone(),
                plan_reasoning_effort: state.form.plan_reasoning_effort.clone(),
                supports_websockets: state.form.supports_websockets,
            };
            let api_key = state.form.api_key.clone();
            perform(
                move || {
                    aam_switcher::provider_secret_store()
                        .map_err(|e| e.to_string())?
                        .save(&id, &api_key)
                        .map_err(|e| e.to_string())?;
                    ProviderRegistry::open_default().upsert(record).map_err(|e| e.to_string())?;
                    Ok(id)
                },
                Message::Saved,
            )
        }
        Message::Saved(Ok(id)) => {
            state.saving = false;
            state.status_message = Some(format!("provider '{id}' 已保存"));
            state.form = NewProviderForm::default();
            load()
        }
        Message::Saved(Err(e)) => {
            state.saving = false;
            state.status_message = Some(format!("保存 provider 失败: {e}"));
            Task::none()
        }
    }
}

pub fn view(state: &State) -> Element<'_, Message> {
    let mut list = column![].spacing(8);
    if state.providers.is_empty() {
        list = list.push(text("(还没有 Provider)"));
    }
    for p in &state.providers {
        list = list.push(
            row![
                text(p.id.clone()).width(Length::Fixed(140.0)),
                text(p.kind.to_string()).width(Length::Fixed(120.0)),
                text(p.base_url.clone()).width(Length::Fill),
                text(format!("model={}", p.model)).width(Length::Fixed(200.0)),
            ]
            .spacing(8),
        );
    }

    let form = &state.form;
    let new_provider_form = column![
        text("新增 Provider").size(18),
        row![
            button(text("cpa")).on_press(Message::KindChanged(ProviderKind::Cpa)),
            button(text("deepseek-v4-flash")).on_press(Message::KindChanged(ProviderKind::DeepseekV4Flash)),
            text(format!("当前选择: {}", form.kind)),
        ]
        .spacing(8),
        text_input("id (留空则用 kind 名字)", &form.id).on_input(Message::IdChanged),
        text_input("base_url", &form.base_url).on_input(Message::BaseUrlChanged),
        model_field(form),
        text_input("API key", &form.api_key).on_input(Message::ApiKeyChanged).secure(true),
        checkbox(form.supports_websockets)
            .label("supports_websockets")
            .on_toggle(Message::SupportsWebsocketsToggled),
        text_input("reasoning_effort", &form.reasoning_effort).on_input(Message::ReasoningEffortChanged),
        text_input("plan_reasoning_effort", &form.plan_reasoning_effort).on_input(Message::PlanReasoningEffortChanged),
        button(text(if state.saving { "保存中..." } else { "保存" })).on_press_maybe(if state.saving {
            None
        } else {
            Some(Message::Submit)
        }),
    ]
    .spacing(8);

    let status = state
        .status_message
        .as_ref()
        .map(|m| text(m.clone()))
        .unwrap_or_else(|| text(""));

    container(
        column![
            text("Providers").size(24),
            text("从不显示已保存的 API key -- 和 CLI 的 `provider list` 一致").size(12),
            iced::widget::scrollable(list).height(Length::FillPortion(3)),
            new_provider_form,
            status,
        ]
        .spacing(16),
    )
    .padding(16)
    .into()
}

fn model_field(form: &NewProviderForm) -> Element<'_, Message> {
    match form.kind {
        ProviderKind::Cpa => text_input("model (必填)", &form.model).on_input(Message::ModelChanged).into(),
        ProviderKind::DeepseekV4Flash => text("model: deepseek-v4-flash (固定型号)").into(),
    }
}
