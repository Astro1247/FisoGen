use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;
use iced::widget::{
    Column, Row, button, column, container, pick_list, row, rule, scrollable, text, text_input,
};
use iced::{Alignment, Element, Fill, FillPortion, Task};

use super::{ACCENT, ERROR, Message, Studio, card, design, field, label, title};
use crate::{
    pricing::{self, COMPONENTS, MODES},
    storage::Record,
};

const ALL: &str = "All";
const PAGE: usize = 8;

#[derive(Debug, Clone, Copy)]
pub enum DetailAction {
    SelectImage(usize),
    ToggleDetails,
    SavedImage,
    ExportRecord,
}

#[derive(Debug, Clone)]
pub enum Action {
    Query(String),
    Model(String),
    Mode(String),
    Quality(String),
    Page(usize),
    Select(usize),
    Detail(DetailAction),
    ExportCsv,
    ExportJsonl,
}

pub struct Gallery {
    query: String,
    model: String,
    mode: String,
    quality: String,
    page: usize,
    selected: Option<usize>,
    image_index: usize,
    details: bool,
}

impl Default for Gallery {
    fn default() -> Self {
        Self {
            query: String::new(),
            model: ALL.into(),
            mode: ALL.into(),
            quality: ALL.into(),
            page: 0,
            selected: None,
            image_index: 0,
            details: false,
        }
    }
}

fn msg(action: Action) -> Message {
    Message::Gallery(action)
}

impl Studio {
    fn filtered(&self) -> Vec<usize> {
        let filters = &self.gallery;
        let search = filters.query.trim().to_lowercase();
        self.history
            .iter()
            .enumerate()
            .filter_map(|(index, record)| {
                let meta = &record.metadata;
                ((search.is_empty()
                    || meta.prompt.to_lowercase().contains(&search)
                    || meta.model.to_lowercase().contains(&search))
                    && (!self.pro
                        || ((filters.model == ALL || filters.model == meta.model)
                            && (filters.mode == ALL || filters.mode == meta.mode)
                            && (filters.quality == ALL || filters.quality == meta.quality))))
                    .then_some(index)
            })
            .collect()
    }

    fn gallery_selection(&self, indices: &[usize]) -> Option<usize> {
        self.gallery
            .selected
            .filter(|index| indices.contains(index))
            .or_else(|| indices.first().copied())
    }

    pub(super) fn view_gallery(&self) -> Element<'_, Message> {
        let indices = self.filtered();
        let selected = self.gallery_selection(&indices);
        let page_count = indices.len().div_ceil(PAGE).max(1);
        let page = self.gallery.page.min(page_count - 1);
        let image_count: usize = indices
            .iter()
            .map(|index| self.history[*index].files.len())
            .sum();
        let complete: Vec<f64> = indices
            .iter()
            .filter_map(|index| self.history[*index].total())
            .collect();
        let spend = self.money((!complete.is_empty()).then(|| complete.iter().sum()));
        let mut toolbar = row![
            title(self.tr("Gallery & History")),
            iced::widget::space::horizontal()
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        if self.pro {
            toolbar = toolbar
                .push(
                    button(self.tr("Export CSV"))
                        .style(design::secondary)
                        .on_press_maybe(
                            (!self.busy && !indices.is_empty()).then_some(msg(Action::ExportCsv)),
                        ),
                )
                .push(
                    button(self.tr("Export JSONL"))
                        .style(design::secondary)
                        .on_press_maybe(
                            (!self.busy && !indices.is_empty()).then_some(msg(Action::ExportJsonl)),
                        ),
                );
        }
        let mut heading = column![
            toolbar,
            text_input(self.tr("Search by prompt or model"), &self.gallery.query)
                .on_input(|query| msg(Action::Query(query)))
                .padding(8),
            label(format!(
                "{} {}  ·  {} {}  ·  {} {}",
                indices.len(),
                self.tr("requests"),
                image_count,
                self.tr("saved images"),
                spend,
                self.tr("complete spend")
            )),
        ]
        .spacing(9);
        if self.pro {
            let options = |f: fn(&Record) -> &str| {
                let mut values: Vec<_> = self
                    .history
                    .iter()
                    .map(|record| f(record).to_owned())
                    .filter(|v| !v.is_empty())
                    .collect();
                values.sort();
                values.dedup();
                values.insert(0, ALL.into());
                values
                    .into_iter()
                    .map(|value| self.choice(value))
                    .collect::<Vec<_>>()
            };
            heading = heading.push(
                row![
                    field(
                        self.tr("Model"),
                        pick_list(
                            options(|r| &r.metadata.model),
                            Some(self.choice(self.gallery.model.clone())),
                            |value| msg(Action::Model(value.value))
                        )
                        .width(Fill)
                    ),
                    field(
                        self.tr("Processing"),
                        pick_list(
                            [ALL, MODES[0], MODES[1]].map(|v| self.choice(v.to_owned())),
                            Some(self.choice(self.gallery.mode.clone())),
                            |value| msg(Action::Mode(value.value))
                        )
                        .width(Fill)
                    ),
                    field(
                        self.tr("Quality"),
                        pick_list(
                            options(|r| &r.metadata.quality),
                            Some(self.choice(self.gallery.quality.clone())),
                            |value| msg(Action::Quality(value.value))
                        )
                        .width(Fill)
                    ),
                ]
                .spacing(8),
            );
        }
        let mut tiles = Column::new().spacing(8);
        for pair in indices
            .iter()
            .skip(page * PAGE)
            .take(PAGE)
            .copied()
            .collect::<Vec<_>>()
            .chunks(2)
        {
            let mut line = Row::new().spacing(8);
            for &index in pair {
                let record = &self.history[index];
                let caption: String = record
                    .metadata
                    .prompt
                    .replace('\n', " ")
                    .chars()
                    .take(58)
                    .collect();
                let tile = column![
                    self.preview_widget(record.files.first().map(String::as_str), 240, 105.0),
                    text(caption).size(14),
                    label(format!(
                        "{}  ·  {}",
                        short_time(&record.time),
                        self.money(record.total())
                    ))
                ]
                .spacing(5);
                line = line.push(
                    button(tile)
                        .padding(7)
                        .width(Fill)
                        .style(if selected == Some(index) {
                            design::selected
                        } else {
                            design::quiet
                        })
                        .on_press(msg(Action::Select(index))),
                );
            }
            if pair.len() == 1 {
                line = line.push(iced::widget::space::horizontal());
            }
            tiles = tiles.push(line);
        }
        if indices.is_empty() {
            tiles = tiles.push(label(if self.history.is_empty() {
                self.tr("Generated images will appear here.")
            } else {
                self.tr("No requests match the current filters.")
            }));
        }
        let list = column![
            heading,
            scrollable(tiles).height(Fill),
            row![
                button(self.tr("Previous"))
                    .style(design::secondary)
                    .on_press_maybe((page > 0).then(|| msg(Action::Page(page - 1)))),
                label(format!("{} / {page_count}", page + 1)),
                button(self.tr("Next"))
                    .style(design::secondary)
                    .on_press_maybe((page + 1 < page_count).then_some(msg(Action::Page(page + 1))))
            ]
            .spacing(8)
            .align_y(Alignment::Center)
        ]
        .spacing(11)
        .height(Fill);
        let detail = if let Some(index) = selected {
            self.record_details(
                &self.history[index],
                self.gallery.image_index,
                self.gallery.details,
                |action| msg(Action::Detail(action)),
            )
        } else {
            column![
                title(self.tr("Request details")),
                label(self.tr("Select a saved request to inspect it."))
            ]
            .spacing(8)
            .into()
        };
        row![
            container(card(list).height(Fill)).width(FillPortion(5)),
            container(scrollable(card(detail)).height(Fill)).width(FillPortion(6))
        ]
        .spacing(14)
        .height(Fill)
        .into()
    }

    pub(super) fn record_details<'a>(
        &'a self,
        record: &'a Record,
        image_index: usize,
        details: bool,
        action: impl Fn(DetailAction) -> Message,
    ) -> Element<'a, Message> {
        let meta = &record.metadata;
        let mut display = column![
            title(self.tr("Request details")),
            label(format!(
                "{}  ·  {}  ·  {}",
                short_time(&record.time),
                meta.model,
                self.choice(meta.mode.as_str())
            )),
            self.preview_widget(
                record.files.get(image_index).map(String::as_str),
                900,
                275.0
            )
        ]
        .spacing(10);
        if !record.files.is_empty() {
            display = display.push(
                row![
                    button(self.tr("Previous image"))
                        .style(design::secondary)
                        .on_press_maybe(
                            (image_index > 0)
                                .then(|| action(DetailAction::SelectImage(image_index - 1)))
                        ),
                    label(format!("{} / {}", image_index + 1, record.files.len())),
                    button(self.tr("Next image"))
                        .style(design::secondary)
                        .on_press_maybe(
                            (image_index + 1 < record.files.len())
                                .then(|| action(DetailAction::SelectImage(image_index + 1)))
                        ),
                    iced::widget::space::horizontal(),
                    button(self.tr("Save a copy"))
                        .style(design::quiet)
                        .on_press_maybe((!self.busy).then(|| action(DetailAction::SavedImage))),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            );
        }
        let average = record
            .total()
            .filter(|_| record.image_count() != 0)
            .map(|cost| cost / record.image_count() as f64);
        display = display.push(
            row![
                column![
                    label(self.tr("Request cost")),
                    text(self.money(record.total())).size(23).color(ACCENT)
                ]
                .width(Fill),
                column![
                    label(self.tr("Cost per returned image")),
                    text(self.money(average)).size(18)
                ]
                .width(Fill),
                column![
                    label(self.tr("Saved / requested")),
                    text(format!("{} / {}", record.files.len(), meta.requested)).size(18)
                ]
                .width(Fill),
            ]
            .spacing(6),
        );
        if !record.error.is_null() {
            display = display.push(text(record.error.to_string()).size(14).color(ERROR));
        }
        for error in &record.output_errors {
            display = display.push(text(error).size(14).color(ERROR));
        }
        display = display
            .push(rule::horizontal(1))
            .push(text(&meta.prompt).size(16));
        if self.pro || details {
            display = display.push(label(format!(
                "{}  ·  {}  ·  {}  ·  {}",
                meta.size,
                self.choice(meta.quality.as_str()),
                meta.format,
                self.choice(meta.background.as_str())
            )));
            if !meta.references.is_empty() {
                display = display.push(label(meta.references.join(", ")));
            }
            if let Some(cost) = &record.cost {
                display = display.push(cost_cells([
                    self.tr("Component").into(),
                    self.tr("Tokens").into(),
                    self.tr("USD / 1M").into(),
                    "USD".into(),
                ]));
                for line in &cost.lines {
                    display = display.push(cost_cells([
                        self.tr_component(&line.component).into(),
                        line.tokens
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| "—".into()),
                        line.rate
                            .map(|r| format!("${r}"))
                            .unwrap_or_else(|| "—".into()),
                        self.money(line.usd),
                    ]));
                }
                if cost.lines.is_empty() {
                    for (name, amount) in [
                        ("Text input", cost.text),
                        ("Image input", cost.image),
                        ("Image output", cost.output),
                    ] {
                        display = display.push(cost_cells([
                            self.tr(name).into(),
                            "—".into(),
                            "—".into(),
                            self.money(amount),
                        ]));
                    }
                }
                display = display.push(label(format!(
                    "{}: {} ({:.2}%)",
                    self.tr("Cost adjustment"),
                    self.money(Some(cost.uplift)),
                    cost.uplift_pct
                )));
                if cost.total.is_none() {
                    display = display.push(label(format!(
                        "{}: {}",
                        self.tr("Known components only"),
                        self.money(Some(cost.known_total))
                    )));
                }
                if let Some(warning) = &cost.warning {
                    display = display.push(label(warning));
                }
            }
            if let (Some(standard), Some(batch)) = (
                meta.comparison_rates.get("Standard"),
                meta.comparison_rates.get("Batch"),
            ) {
                let eligible = meta.model.starts_with("gpt-image-2.6");
                let total = |rates| {
                    pricing::estimate(&record.usage, rates, eligible, meta.uplift_pct)
                        .and_then(|c| c.total)
                };
                if let Some((standard, batch)) = total(standard).zip(total(batch)) {
                    display = display.push(label(format!(
                        "{}: {} / {}  ·  {} {}",
                        self.tr("Same usage at Standard / Batch rates"),
                        self.money(Some(standard)),
                        self.money(Some(batch)),
                        self.tr("difference"),
                        self.money(Some(standard - batch))
                    )));
                }
            }
        }
        display = display.push(
            row![
                button(if details {
                    self.tr("Hide raw details")
                } else {
                    self.tr("Show raw details")
                })
                .style(design::secondary)
                .on_press(action(DetailAction::ToggleDetails)),
                button(self.tr("Export request JSON"))
                    .style(design::quiet)
                    .on_press_maybe((!self.busy).then(|| action(DetailAction::ExportRecord))),
            ]
            .spacing(7),
        );
        if details {
            display = display.push(
                text(serde_json::to_string_pretty(record).unwrap_or_default())
                    .font(design::MONO_FONT)
                    .size(14),
            );
        }
        display.into()
    }

    pub(super) fn gallery_previews(&mut self) -> Task<Message> {
        if self.tab != super::Tab::Gallery {
            return Task::none();
        }
        let indices = self.filtered();
        let page = self
            .gallery
            .page
            .min(indices.len().div_ceil(PAGE).max(1) - 1);
        let mut paths: Vec<(PathBuf, u32)> = indices
            .iter()
            .skip(page * PAGE)
            .take(PAGE)
            .filter_map(|index| {
                self.history[*index]
                    .files
                    .first()
                    .and_then(|name| self.store.image_path(name))
            })
            .map(|path| (path, 240))
            .collect();
        if let Some(path) = self
            .gallery_selection(&indices)
            .and_then(|index| self.history[index].files.get(self.gallery.image_index))
            .and_then(|name| self.store.image_path(name))
        {
            paths.push((path, 900));
        }
        self.previews(paths)
    }

    pub(super) fn preserve_gallery_selection(&mut self, history: &[Record]) {
        if let Some(index) = self.gallery.selected {
            self.gallery.selected = self.history.get(index).and_then(|old| {
                history
                    .iter()
                    .position(|record| record.record_id == old.record_id)
            });
            if self.gallery.selected.is_none() {
                self.gallery.image_index = 0;
            }
        }
    }

    pub(super) fn update_gallery(&mut self, action: Action) -> Task<Message> {
        let gallery = &mut self.gallery;
        match action {
            Action::Query(query) => {
                gallery.query = query;
                gallery.page = 0;
                gallery.selected = None;
                gallery.image_index = 0;
            }
            Action::Model(model) => {
                gallery.model = model;
                gallery.page = 0;
                gallery.selected = None;
                gallery.image_index = 0;
            }
            Action::Mode(mode) => {
                gallery.mode = mode;
                gallery.page = 0;
                gallery.selected = None;
                gallery.image_index = 0;
            }
            Action::Quality(quality) => {
                gallery.quality = quality;
                gallery.page = 0;
                gallery.selected = None;
                gallery.image_index = 0;
            }
            Action::Page(page) => gallery.page = page,
            Action::Select(index) => {
                gallery.selected = Some(index);
                gallery.image_index = 0;
            }
            Action::Detail(DetailAction::SelectImage(index)) => gallery.image_index = index,
            Action::Detail(DetailAction::ToggleDetails) => gallery.details = !gallery.details,
            Action::Detail(action) => {
                if let Some(index) = self.gallery_selection(&self.filtered()) {
                    return self.export_detail(
                        action,
                        self.history[index].clone(),
                        self.gallery.image_index,
                    );
                }
            }
            Action::ExportCsv | Action::ExportJsonl => {
                let records: Vec<_> = self
                    .filtered()
                    .into_iter()
                    .map(|index| self.history[index].clone())
                    .collect();
                if matches!(action, Action::ExportCsv) {
                    return self.file_task(
                        "image_cost_history.csv",
                        "CSV",
                        &["csv"],
                        move |path| write_csv(path, &records),
                    );
                }
                return self.file_task(
                    "image_cost_history.jsonl",
                    "JSON Lines",
                    &["jsonl"],
                    move |path| {
                        use std::io::Write;
                        let mut file = std::io::BufWriter::new(fs::File::create(path)?);
                        for record in records {
                            serde_json::to_writer(&mut file, &record)?;
                            file.write_all(b"\n")?;
                        }
                        file.flush()?;
                        Ok(())
                    },
                );
            }
        }
        self.gallery_previews()
    }

    pub(super) fn export_detail(
        &mut self,
        action: DetailAction,
        record: Record,
        index: usize,
    ) -> Task<Message> {
        match action {
            DetailAction::SavedImage => {
                let Some(filename) = record.files.get(index) else {
                    return Task::none();
                };
                let Some(source) = self.store.image_path(filename) else {
                    return Task::none();
                };
                let ext = source
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .unwrap_or("png");
                let copy = source.clone();
                self.file_task(filename, self.tr("Image"), &[ext], move |path| {
                    if fs::canonicalize(path)
                        .ok()
                        .zip(fs::canonicalize(&copy).ok())
                        .is_none_or(|(a, b)| a != b)
                    {
                        fs::copy(copy, path)?;
                    }
                    Ok(())
                })
            }
            DetailAction::ExportRecord => {
                self.file_task("image_cost_details.json", "JSON", &["json"], move |path| {
                    fs::write(path, serde_json::to_vec_pretty(&record)?)?;
                    Ok(())
                })
            }
            _ => Task::none(),
        }
    }
}

fn cost_cells(values: [String; 4]) -> Row<'static, Message> {
    let [name, tokens, rate, cost] = values;
    row![
        label(name).width(FillPortion(2)),
        label(tokens).width(Fill),
        label(rate).width(Fill),
        label(cost).width(Fill)
    ]
    .spacing(5)
}

pub(super) fn short_time(time: &str) -> String {
    time.chars().take(19).collect::<String>().replace('T', " ")
}

fn write_csv(path: &Path, records: &[Record]) -> Result<()> {
    let mut writer = csv::Writer::from_path(path)?;
    let mut headings = vec![
        "Time (UTC)",
        "Prompt",
        "Model",
        "Processing",
        "Quality",
        "Size",
        "References",
        "Images saved",
        "Images requested",
        "Text tokens",
        "Image input tokens",
        "Output tokens",
        "Total tokens",
        "Text cost USD",
        "Image input cost USD",
        "Output cost USD",
        "Adjustment USD",
        "Total USD",
        "Cost complete",
        "Pricing basis",
        "Prices checked",
        "Seconds",
        "Batch ID",
        "Error",
    ];
    headings.extend(COMPONENTS.iter().map(|(_, label)| *label));
    writer.write_record(headings)?;
    for record in records {
        let meta = &record.metadata;
        let cost = record.cost.as_ref();
        let usd = |value: Option<f64>| value.map(|n| n.to_string()).unwrap_or_default();
        let value = |v: &serde_json::Value| {
            if v.is_null() {
                String::new()
            } else {
                v.as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| v.to_string())
            }
        };
        let mut row = vec![
            record.time.clone(),
            meta.prompt.clone(),
            meta.model.clone(),
            meta.mode.clone(),
            meta.quality.clone(),
            meta.size.clone(),
            meta.references.len().to_string(),
            record.files.len().to_string(),
            meta.requested.to_string(),
            value(&record.usage["input_tokens_details"]["text_tokens"]),
            value(&record.usage["input_tokens_details"]["image_tokens"]),
            value(&record.usage["output_tokens"]),
            value(&record.usage["total_tokens"]),
            usd(cost.and_then(|c| c.text)),
            usd(cost.and_then(|c| c.image)),
            usd(cost.and_then(|c| c.output)),
            usd(cost.map(|c| c.uplift)),
            usd(record.total()),
            record.total().is_some().to_string(),
            meta.pricing_basis.clone(),
            meta.pricing_checked.clone(),
            record
                .details
                .get("elapsed_s")
                .map(value)
                .unwrap_or_default(),
            record
                .details
                .get("batch_id")
                .map(value)
                .unwrap_or_default(),
            value(&record.error),
        ];
        row.extend(
            COMPONENTS
                .iter()
                .map(|(key, _)| usd(meta.rates.get(*key).copied())),
        );
        writer.write_record(row)?;
    }
    writer.flush()?;
    Ok(())
}
