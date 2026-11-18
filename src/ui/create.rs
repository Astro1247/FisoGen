use std::{
    fs::File,
    io::{BufWriter, Write},
    path::PathBuf,
};

use anyhow::{Result, ensure};
use iced::widget::{
    Column, button, checkbox, column, container, pick_list, row, scrollable, text, text_editor,
    text_input,
};
use iced::{Element, Fill, FillPortion, Task};

use super::{
    MUTED, Message, Outcome, Studio, asset_name, card, design, field, gallery, label, title,
};
use crate::{
    api::{self, DraftEntry},
    pricing::{MODELS, MODES, PRICING_URL},
    storage::{Metadata, Record},
};

const SIZES: [&str; 6] = [
    "1024x1024",
    "1024x1536",
    "1536x1024",
    "1536x864",
    "2048x2048",
    "Custom",
];
const QUALITIES: [&str; 6] = ["auto", "low", "medium", "high", "xhigh", "max"];
const FORMATS: [&str; 3] = ["png", "webp", "jpeg"];
const BACKGROUNDS: [&str; 3] = ["transparent", "opaque", "auto"];
const COMPLEXITY: [&str; 3] = ["high", "medium", "low"];

#[derive(Clone, Debug)]
pub enum Action {
    Prompt(text_editor::Action),
    Model(&'static str),
    Mode(&'static str),
    Size(&'static str),
    CustomSize(String),
    Quality(&'static str),
    Format(&'static str),
    Background(&'static str),
    Complexity(&'static str),
    Count(String),
    PickReferences,
    ClearReferences,
    Confirm(bool),
    Generate,
    AddToDraft,
    SelectDraft(usize),
    RemoveDraft,
    ExportDraft,
    Submit,
    Detail(gallery::DetailAction),
}

pub struct Create {
    prompt: text_editor::Content,
    pub model: &'static str,
    mode: &'static str,
    size: &'static str,
    custom_size: String,
    quality: &'static str,
    format: &'static str,
    background: &'static str,
    complexity: &'static str,
    count: String,
    pub references: Vec<PathBuf>,
    pub confirm: bool,
    pub draft: Vec<DraftEntry>,
    pub draft_selected: usize,
    pub latest: Option<Record>,
    pub image_index: usize,
    show_details: bool,
}

impl Default for Create {
    fn default() -> Self {
        Self {
            prompt: text_editor::Content::new(),
            model: MODELS[0],
            mode: "Standard",
            size: SIZES[0],
            custom_size: "1536x1024".into(),
            quality: "high",
            format: "png",
            background: "transparent",
            complexity: "high",
            count: "1".into(),
            references: Vec::new(),
            confirm: false,
            draft: Vec::new(),
            draft_selected: 0,
            latest: None,
            image_index: 0,
            show_details: false,
        }
    }
}

fn msg(action: Action) -> Message {
    Message::Create(action)
}

impl Studio {
    fn batch_mode(&self) -> bool {
        self.pro && self.create.mode == "Batch"
    }

    pub(super) fn view_create(&self) -> Element<'_, Message> {
        let form = &self.create;
        let batch = self.batch_mode();
        let mut inputs = column![
            title(self.tr(if batch {
                "Queue an image request"
            } else {
                "Create an image"
            })),
            self.key_input(),
            label(self.tr(if batch { "Prompts" } else { "Prompt" })),
            text_editor(&form.prompt)
                .placeholder(self.tr("Describe the image you want to create…"))
                .height(135)
                .padding(12)
                .size(17)
                .line_height(1.4)
                .style(design::editor)
                .on_action(|action| msg(Action::Prompt(action))),
            row![
                field(
                    self.tr("Model"),
                    pick_list(MODELS, Some(form.model), |m| msg(Action::Model(m))).width(Fill)
                ),
                field(
                    self.tr("Dimensions"),
                    pick_list(
                        SIZES.map(|s| self.choice(s)),
                        Some(self.choice(form.size)),
                        |s| msg(Action::Size(s.value))
                    )
                    .width(Fill)
                )
            ]
            .spacing(10),
        ]
        .spacing(10);
        if batch {
            inputs = inputs.push(label(
                self.tr("Separate prompts with a line containing only ---."),
            ));
        }
        if form.size == "Custom" {
            inputs = inputs.push(field(
                self.tr("Width x height"),
                text_input("1536x1024", &form.custom_size).on_input(|s| msg(Action::CustomSize(s))),
            ));
        }
        let refs = row![
            button(self.tr(if form.references.is_empty() {
                "Add references"
            } else {
                "Change references"
            }))
            .style(design::secondary)
            .on_press_maybe((!self.busy).then_some(msg(Action::PickReferences))),
            button(self.tr("Clear"))
                .style(design::quiet)
                .on_press_maybe(
                    (!form.references.is_empty() && !self.busy)
                        .then_some(msg(Action::ClearReferences))
                )
        ]
        .spacing(5);
        inputs = inputs.push(refs);
        if !form.references.is_empty() {
            let names = form
                .references
                .iter()
                .take(3)
                .map(|path| asset_name(path))
                .collect::<Vec<_>>()
                .join(", ");
            let more = if form.references.len() > 3 {
                format!(" +{}", form.references.len() - 3)
            } else {
                String::new()
            };
            inputs = inputs.push(label(self.translate_format(
                "Editing with {references}",
                &[("references", format!("{names}{more}"))],
            )));
        }
        if self.pro {
            inputs = inputs.push(
                row![
                    field(
                        self.tr("Processing"),
                        pick_list(
                            MODES.map(|m| self.choice(m)),
                            Some(self.choice(form.mode)),
                            |m| msg(Action::Mode(m.value))
                        )
                        .width(Fill)
                    ),
                    field(
                        self.tr("Quality"),
                        pick_list(
                            QUALITIES.map(|q| self.choice(q)),
                            Some(self.choice(form.quality)),
                            |q| msg(Action::Quality(q.value))
                        )
                        .width(Fill)
                    ),
                    field(
                        self.tr("Images / request"),
                        text_input("1–10", &form.count).on_input(|n| msg(Action::Count(n)))
                    ),
                ]
                .spacing(10),
            );
            let mut options = row![
                field(
                    self.tr("Background"),
                    pick_list(
                        BACKGROUNDS.map(|b| self.choice(b)),
                        Some(self.choice(form.background)),
                        |b| msg(Action::Background(b.value))
                    )
                    .width(Fill)
                ),
                field(
                    self.tr("Format"),
                    pick_list(FORMATS, Some(form.format), |f| msg(Action::Format(f))).width(Fill)
                ),
            ]
            .spacing(10);
            if form.model.starts_with("gpt-image-2.6") {
                options = options.push(field(
                    self.tr("Complexity"),
                    pick_list(
                        COMPLEXITY.map(|c| self.choice(c)),
                        Some(self.choice(form.complexity)),
                        |c| msg(Action::Complexity(c.value)),
                    )
                    .width(Fill),
                ));
            }
            inputs = inputs.push(options);
        }
        let has_prompt = !form.prompt.text().trim().is_empty();
        if batch {
            inputs = inputs.push(
                button(self.tr("Add to Batch draft"))
                    .padding([9, 14])
                    .style(design::primary)
                    .on_press_maybe((has_prompt && !self.busy).then_some(msg(Action::AddToDraft))),
            );
        } else {
            inputs = inputs
                .push(
                    checkbox(form.confirm)
                        .label(self.tr("I confirm this billable request"))
                        .on_toggle(|value| msg(Action::Confirm(value))),
                )
                .push(
                    button(self.tr(if form.references.is_empty() {
                        "Generate image"
                    } else {
                        "Generate edit"
                    }))
                    .padding([10, 18])
                    .style(design::primary)
                    .on_press_maybe(
                        (!self.busy && form.confirm && has_prompt && !self.key.trim().is_empty())
                            .then_some(msg(Action::Generate)),
                    ),
                );
        }

        let result =
            if batch {
                self.view_draft()
            } else if let Some(record) = &form.latest {
                self.record_details(record, form.image_index, form.show_details, |action| {
                    msg(Action::Detail(action))
                })
            } else {
                column![
                    title(self.tr("Result preview")),
                    self.preview_widget(None, 900, 290.0),
                    label(self.tr(
                        "Generated images and their costs appear here and in Gallery & History."
                    ))
                ]
                .spacing(12)
                .into()
            };
        scrollable(
            row![
                container(card(inputs)).width(FillPortion(5)),
                container(card(result)).width(FillPortion(6))
            ]
            .spacing(14)
            .align_y(iced::Alignment::Start),
        )
        .height(Fill)
        .into()
    }

    fn view_draft(&self) -> Element<'_, Message> {
        let form = &self.create;
        let size: usize = form.draft.iter().map(|entry| entry.line.len()).sum();
        let images: u64 = form
            .draft
            .iter()
            .map(|entry| entry.metadata.requested)
            .sum();
        let mut panel = column![
            title(self.tr("Batch draft")),
            label(format!(
                "{} {}  ·  {images} {}  ·  {:.2} MB",
                form.draft.len(),
                self.tr("requests"),
                self.tr("images"),
                size as f64 / 1_000_000.0
            ))
        ]
        .spacing(11);
        if form.draft.is_empty() {
            return panel
                .push(text(self.tr("Add requests to the draft before submitting.")).color(MUTED))
                .into();
        }
        let entries = Column::with_children(form.draft.iter().enumerate().map(|(index, entry)| {
            let prompt: String = entry
                .metadata
                .prompt
                .replace('\n', " ")
                .chars()
                .take(72)
                .collect();
            button(text(format!("{}. {prompt}", index + 1)).size(14))
                .width(Fill)
                .style(if index == form.draft_selected {
                    design::secondary
                } else {
                    design::quiet
                })
                .on_press(msg(Action::SelectDraft(index)))
                .into()
        }))
        .spacing(3);
        panel = panel.push(scrollable(entries).height(155));
        if let Some(entry) = form.draft.get(form.draft_selected) {
            let meta = &entry.metadata;
            panel = panel
                .push(text(&meta.prompt).size(15))
                .push(label(format!(
                    "{}  ·  {}  ·  {}  ·  {}  ·  {} {}",
                    meta.model,
                    meta.size,
                    self.choice(meta.quality.as_str()),
                    meta.format,
                    meta.requested,
                    self.tr("images")
                )))
                .push(label(format!(
                    "{}  ·  {} {}  ·  {}",
                    entry.endpoint,
                    meta.references.len(),
                    self.tr("references"),
                    meta.pricing_basis
                )))
                .push(label(format!(
                    "{}  ·  {}",
                    self.tr("USD / million tokens"),
                    crate::pricing::COMPONENTS
                        .iter()
                        .filter_map(|(key, name)| {
                            meta.rates
                                .get(*key)
                                .map(|v| format!("{} ${v}", self.tr(name)))
                        })
                        .collect::<Vec<_>>()
                        .join("  ·  ")
                )));
        }
        panel.push(row![button(self.tr("Remove selected")).style(design::secondary).on_press_maybe((!self.busy).then_some(msg(Action::RemoveDraft))),
                button(self.tr("Export draft JSONL")).style(design::secondary).on_press_maybe((!self.busy).then_some(msg(Action::ExportDraft)))].spacing(8))
            .push(label(self.tr("Only the listed requests will be submitted. A Batch uses one model and one request type.")))
            .push(checkbox(form.confirm).label(self.tr("I confirm these billable requests")).on_toggle(|v| msg(Action::Confirm(v))))
            .push(button(self.tr("Submit Batch job")).padding([10, 16]).style(design::primary).on_press_maybe((!self.busy && form.confirm && !self.key.trim().is_empty()).then_some(msg(Action::Submit))))
            .into()
    }

    fn generation_metadata(&self) -> Result<Metadata> {
        let form = &self.create;
        let prompt = form.prompt.text().trim().to_owned();
        ensure!(!prompt.is_empty(), "{}", self.tr("Enter a prompt first."));
        let size = if form.size == "Custom" {
            form.custom_size.trim()
        } else {
            form.size
        };
        ensure!(
            size.split_once('x')
                .is_some_and(|(w, h)| [w, h].into_iter().all(|side| {
                    (2..=5).contains(&side.len())
                        && !side.starts_with('0')
                        && side.bytes().all(|b| b.is_ascii_digit())
                })),
            "{}",
            self.tr("Enter dimensions as width x height, for example 1536x1024.")
        );
        let (quality, format, background, count, complexity) = if self.pro {
            (
                form.quality,
                form.format,
                form.background,
                form.count.as_str(),
                form.complexity,
            )
        } else {
            ("high", "png", "auto", "1", "high")
        };
        let requested = count.parse::<u64>().ok().filter(|n| (1..=10).contains(n));
        let Some(requested) = requested else {
            anyhow::bail!("{}", self.tr("Enter 1 to 10 images per request."));
        };
        ensure!(
            background != "transparent" || format != "jpeg",
            "{}",
            self.tr("Choose PNG or WebP for transparency.")
        );
        ensure!(
            form.references.len() <= 16,
            "{}",
            self.tr("Choose up to 16 reference images.")
        );
        let mode = if self.batch_mode() {
            "Batch"
        } else {
            "Standard"
        };
        let comparison = self.comparison(form.model).clone();
        let overridden = self.overrides.contains_key(form.model);
        Ok(Metadata {
            model: form.model.into(),
            prompt,
            size: size.into(),
            quality: quality.into(),
            background: background.into(),
            format: format.into(),
            complexity: form
                .model
                .starts_with("gpt-image-2.6")
                .then(|| complexity.into()),
            references: form
                .references
                .iter()
                .map(|path| asset_name(path))
                .collect(),
            requested,
            mode: mode.into(),
            rates: comparison[mode].clone(),
            comparison_rates: comparison,
            pricing_source: PRICING_URL.into(),
            pricing_checked: self.checked.clone(),
            pricing_basis: if overridden {
                "Custom override"
            } else {
                "Official published snapshot"
            }
            .into(),
            uplift_pct: self.planner.applied_uplift,
        })
    }

    pub(super) fn update_create(&mut self, action: Action) -> Task<Message> {
        match action {
            Action::Prompt(action) => self.create.prompt.perform(action),
            Action::Model(model) => {
                self.create.model = model;
                self.cancel_rate_edit();
            }
            Action::Mode(mode) => {
                self.create.mode = mode;
                self.create.confirm = false;
            }
            Action::Size(size) => self.create.size = size,
            Action::CustomSize(size) => self.create.custom_size = size,
            Action::Quality(quality) => self.create.quality = quality,
            Action::Format(format) => self.create.format = format,
            Action::Background(background) => self.create.background = background,
            Action::Complexity(complexity) => self.create.complexity = complexity,
            Action::Count(count) => self.create.count = count,
            Action::Confirm(confirm) => self.create.confirm = confirm,
            Action::ClearReferences => {
                self.create.references.clear();
                self.create.confirm = false;
            }
            Action::PickReferences => {
                let filter = self.tr("Images");
                let limit = self.tr("Choose up to 16 reference images.");
                return self.start(self.tr("Choose reference images."), async move {
                    let handles = rfd::AsyncFileDialog::new()
                        .add_filter(filter, &["png", "jpg", "jpeg", "webp"])
                        .pick_files()
                        .await
                        .unwrap_or_default();
                    ensure!(handles.len() <= 16, "{limit}");
                    Ok(Outcome::References(
                        handles
                            .iter()
                            .map(|handle| handle.path().to_owned())
                            .collect(),
                    ))
                });
            }
            Action::Generate | Action::AddToDraft => {
                if self.busy {
                    return Task::none();
                }
                let metadata = match self.generation_metadata() {
                    Ok(metadata) => metadata,
                    Err(error) => {
                        self.fail(error.to_string());
                        return Task::none();
                    }
                };
                let refs = self.create.references.clone();
                if matches!(action, Action::AddToDraft) {
                    let draft = self.create.draft.clone();
                    self.create.confirm = false;
                    return self.start(self.tr("Preparing the Batch draft…"), async move {
                        let additions = tokio::task::spawn_blocking(move || {
                            api::draft_entries(metadata, &refs, &draft)
                        })
                        .await??;
                        Ok(Outcome::Drafted(additions))
                    });
                }
                if self.key.trim().is_empty() || !self.create.confirm || self.batch_mode() {
                    return Task::none();
                }
                self.create.confirm = false;
                let (api, store, key) = (
                    self.api.clone(),
                    self.store.clone(),
                    self.key.trim().to_owned(),
                );
                return self.start(
                    self.tr("Generating images and saving results automatically…"),
                    async move {
                        Ok(Outcome::Generated(Box::new(
                            api.generate(&store, &key, metadata, refs).await?,
                        )))
                    },
                );
            }
            Action::SelectDraft(index) => self.create.draft_selected = index,
            Action::RemoveDraft => {
                if !self.busy && self.create.draft_selected < self.create.draft.len() {
                    self.create.draft.remove(self.create.draft_selected);
                    self.create.draft_selected = self
                        .create
                        .draft_selected
                        .min(self.create.draft.len().saturating_sub(1));
                    self.create.confirm = false;
                }
            }
            Action::ExportDraft => {
                let draft = self.create.draft.clone();
                return self.file_task(
                    "image_batch_draft.jsonl",
                    "JSON Lines",
                    &["jsonl"],
                    move |path| {
                        let mut output = BufWriter::new(File::create(path)?);
                        for entry in draft {
                            output.write_all(&entry.line)?;
                        }
                        output.flush()?;
                        Ok(())
                    },
                );
            }
            Action::Submit => {
                if !self.batch_mode()
                    || !self.create.confirm
                    || self.key.trim().is_empty()
                    || self.create.draft.is_empty()
                    || self.busy
                {
                    return Task::none();
                }
                self.create.confirm = false;
                let (api, store, key, draft) = (
                    self.api.clone(),
                    self.store.clone(),
                    self.key.trim().to_owned(),
                    self.create.draft.clone(),
                );
                return self.start(
                    self.tr("Uploading the draft and creating the Batch job…"),
                    async move { Ok(Outcome::Submitted(api.submit(&store, &key, &draft).await?)) },
                );
            }
            Action::Detail(action) => match action {
                gallery::DetailAction::SelectImage(index) => {
                    self.create.image_index = index;
                    return self.create_preview();
                }
                gallery::DetailAction::ToggleDetails => {
                    self.create.show_details = !self.create.show_details
                }
                gallery::DetailAction::SavedImage | gallery::DetailAction::ExportRecord => {
                    if let Some(record) = self.create.latest.clone() {
                        return self.export_detail(action, record, self.create.image_index);
                    }
                }
            },
        }
        Task::none()
    }

    pub(super) fn create_preview(&mut self) -> Task<Message> {
        let path = self
            .create
            .latest
            .as_ref()
            .and_then(|record| record.files.get(self.create.image_index))
            .and_then(|name| self.store.image_path(name));
        self.previews(path.into_iter().map(|path| (path, 900)))
    }
}
