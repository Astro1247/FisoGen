use anyhow::{Context, Result, ensure};
use iced::widget::{
    Column, Row, button, column, pick_list, row, rule, scrollable, text, text_input,
};
use iced::{Alignment, Element, Fill, FillPortion, Task};
use serde_json::json;

use super::{ACCENT, Message, Outcome, Studio, card, design, field, label, tab_link, title};
use crate::pricing::{self, COMPONENTS, MODELS, MODES};

const FIELDS: [&str; 6] = [
    "Text tokens / request",
    "Image tokens / request",
    "Output tokens / image",
    "Images / request",
    "Requests",
    "Budget USD",
];

#[derive(Clone, Debug)]
pub enum Action {
    Search(String),
    Model(&'static str),
    Input(usize, String),
    Refresh,
    EditRates,
    Rate(usize, String),
    ApplyRates,
    CancelRates,
    ResetRates,
    Uplift(String),
    ApplyUplift,
}

pub struct Planner {
    search: String,
    inputs: [String; 6],
    editing: Option<[String; 10]>,
    uplift: String,
    pub applied_uplift: f64,
}

impl Default for Planner {
    fn default() -> Self {
        Self {
            search: String::new(),
            inputs: ["0", "0", "0", "1", "100", "10"].map(str::to_owned),
            editing: None,
            uplift: "0".into(),
            applied_uplift: 0.0,
        }
    }
}

fn msg(action: Action) -> Message {
    Message::Planner(action)
}

impl Studio {
    pub(super) fn view_planner(&self) -> Element<'_, Message> {
        let model = self.create.model;
        let planner = &self.planner;
        let mut output = column![
            title(self.tr("Official token prices")),
            row![
                label(format!(
                    "{}  ·  {}",
                    self.tr("USD / million tokens"),
                    self.checked
                )),
                iced::widget::space::horizontal(),
                button(self.tr("Refresh official prices"))
                    .style(design::secondary)
                    .on_press_maybe((!self.busy).then_some(msg(Action::Refresh)))
            ]
            .align_y(Alignment::Center),
            text_input(self.tr("Find a model or processing mode"), &planner.search)
                .on_input(|value| msg(Action::Search(value)))
                .padding(8),
            table_row([
                self.tr("Model and processing").into(),
                self.tr("Text input").into(),
                self.tr("Cached text input").into(),
                self.tr("Image input").into(),
                self.tr("Cached image input").into(),
                self.tr("Image output").into()
            ]),
            rule::horizontal(1),
        ]
        .spacing(9);
        let search = planner.search.trim().to_lowercase();
        for model in MODELS {
            for mode in MODES {
                if !format!("{model} {mode} {}", self.tr(mode))
                    .to_lowercase()
                    .contains(&search)
                {
                    continue;
                }
                let values = &self.catalog[model][mode];
                output = output.push(table_row(std::array::from_fn(|index| {
                    if index == 0 {
                        format!("{model}  ·  {}", self.tr(mode))
                    } else {
                        format!("${}", values[COMPONENTS[index - 1].0])
                    }
                })));
            }
        }
        output = output.push(label(self.tr("GPT Image 2.5 cache discounts apply through the Responses API. This app uses the direct Images API.")))
            .push(row![tab_link(self.tr("Official prices"), pricing::PRICING_URL), tab_link(self.tr("Size & quality calculator"), pricing::CALCULATOR),
                tab_link(self.tr("Caching eligibility"), pricing::CACHE), tab_link(self.tr("Account usage"), "https://platform.openai.com/usage")].spacing(6));

        let mut controls = column![
            title(self.tr("Workload planner")),
            row![
                field(
                    self.tr("Model"),
                    pick_list(MODELS, Some(model), |m| msg(Action::Model(m))).width(Fill)
                ),
                field(
                    self.tr("Custom uplift (%)"),
                    text_input("0", &planner.uplift).on_input(|v| msg(Action::Uplift(v)))
                ),
                button(self.tr("Apply uplift"))
                    .style(design::secondary)
                    .on_press(msg(Action::ApplyUplift)),
            ]
            .spacing(9)
            .align_y(Alignment::End),
            row![
                label(if self.overrides.contains_key(model) {
                    self.tr("Custom rates")
                } else {
                    self.tr("Published rates")
                }),
                button(self.tr("Edit rates"))
                    .style(design::secondary)
                    .on_press(msg(Action::EditRates)),
                button(self.tr("Use published rates"))
                    .style(design::quiet)
                    .on_press_maybe(
                        self.overrides
                            .contains_key(model)
                            .then_some(msg(Action::ResetRates))
                    )
            ]
            .spacing(9)
            .align_y(Alignment::Center),
            label(format!(
                "{}  ·  {:.2}%",
                self.tr("Applied uplift"),
                planner.applied_uplift
            )),
        ]
        .spacing(10);
        if let Some(values) = &planner.editing {
            let editors = MODES
                .into_iter()
                .enumerate()
                .map(|(mode, name)| {
                    let fields = COMPONENTS.iter().enumerate().map(|(index, (_, name))| {
                        let index = mode * COMPONENTS.len() + index;
                        field(
                            self.tr(name),
                            text_input("0", &values[index])
                                .on_input(move |value| msg(Action::Rate(index, value))),
                        )
                        .into()
                    });
                    column![
                        text(self.tr(name)),
                        Column::with_children(fields).spacing(5)
                    ]
                    .spacing(6)
                    .width(Fill)
                })
                .collect::<Vec<_>>();
            controls = controls
                .push(Row::with_children(editors.into_iter().map(Into::into)).spacing(10))
                .push(
                    row![
                        button(self.tr("Apply rates")).on_press(msg(Action::ApplyRates)),
                        button(self.tr("Cancel"))
                            .style(design::secondary)
                            .on_press(msg(Action::CancelRates))
                    ]
                    .spacing(8),
                );
        }

        controls = controls.push(label(self.tr("Enter expected tokens from a prior request or the official calculator. The planner makes no API calls.")));
        for range in [0..3, 3..6] {
            controls = controls.push(
                Row::with_children(range.map(|index| {
                    field(
                        self.tr(FIELDS[index]),
                        text_input("0", &planner.inputs[index])
                            .on_input(move |v| msg(Action::Input(index, v))),
                    )
                    .into()
                }))
                .spacing(9),
            );
        }
        controls = controls.push(self.plan_summary());
        scrollable(column![card(controls), card(output)].spacing(14))
            .height(Fill)
            .into()
    }

    fn plan_summary(&self) -> Element<'_, Message> {
        let fields = &self.planner.inputs;
        let parsed = [0, 1, 2, 3, 4].map(|i| fields[i].trim().parse::<u64>().ok());
        let [
            Some(text_tokens),
            Some(image_tokens),
            Some(output_tokens),
            Some(images),
            Some(requests),
        ] = parsed
        else {
            return label(self.tr("Enter whole numbers for tokens, images, and requests.")).into();
        };
        let Ok(budget) = finite_nonnegative(&fields[5], 1e12) else {
            return label(self.tr("Enter a valid budget.")).into();
        };
        if !(1..=10).contains(&images)
            || !(1..=50_000).contains(&requests)
            || [text_tokens, image_tokens, output_tokens]
                .into_iter()
                .any(|n| n > 1_000_000_000_000)
        {
            return label(self.tr("Enter 1–10 images per request and 1–50,000 requests.")).into();
        }
        if text_tokens + image_tokens + output_tokens == 0 {
            return label(self.tr("Enter token quantities to compare Standard and Batch.")).into();
        }
        let usage = json!({"input_tokens": text_tokens + image_tokens, "output_tokens": output_tokens * images,
            "input_tokens_details": {"text_tokens": text_tokens, "image_tokens": image_tokens}});
        let comparison = self.comparison(self.create.model);
        let costs = MODES.map(|mode| {
            pricing::estimate(
                &usage,
                &comparison[mode],
                false,
                self.planner.applied_uplift,
            )
            .unwrap()
        });
        let total = costs.each_ref().map(|cost| cost.total.unwrap_or_default());
        let mut summary = column![
            rule::horizontal(1),
            row![
                self.plan_metric("Standard workload", total[0] * requests as f64),
                self.plan_metric("Batch workload", total[1] * requests as f64),
                self.plan_metric("Potential savings", (total[0] - total[1]) * requests as f64),
            ]
            .spacing(8),
            table_row([
                self.tr("Processing").into(),
                self.tr("Text input").into(),
                self.tr("Image input").into(),
                self.tr("Image output").into(),
                self.tr("USD / image").into(),
                self.tr("Requests within budget").into()
            ])
        ]
        .spacing(8);
        for (index, mode) in MODES.into_iter().enumerate() {
            let cost = &costs[index];
            summary = summary.push(table_row([
                format!(
                    "{}  ·  {} / {}",
                    self.tr(mode),
                    self.money(cost.total),
                    self.tr("request")
                ),
                self.money(cost.text),
                self.money(cost.image),
                self.money(cost.output),
                self.money(cost.total.map(|v| v / images as f64)),
                if total[index] > 0.0 {
                    format!("{:.0}", (budget / total[index]).floor())
                } else {
                    "—".into()
                },
            ]));
        }
        summary.push(label(self.tr("Cost = reported tokens × rate / 1,000,000. The planner includes only the token quantities entered here."))).into()
    }

    fn plan_metric(&self, name: &'static str, value: f64) -> Column<'static, Message> {
        column![
            label(self.tr(name)),
            text(self.money(Some(value))).size(24).color(ACCENT)
        ]
        .spacing(3)
        .width(Fill)
    }

    pub(super) fn update_planner(&mut self, action: Action) -> Task<Message> {
        match action {
            Action::Search(value) => self.planner.search = value,
            Action::Model(model) => {
                self.create.model = model;
                self.planner.editing = None;
            }
            Action::Input(index, value) => self.planner.inputs[index] = value,
            Action::Uplift(value) => self.planner.uplift = value,
            Action::ApplyUplift => match finite_nonnegative(&self.planner.uplift, 1000.0) {
                Ok(value) => {
                    self.planner.applied_uplift = value;
                    self.notice(self.tr("Applied the cost adjustment for this session."));
                }
                Err(_) => self.fail(self.tr("Enter an uplift between 0 and 1,000 percent.")),
            },
            Action::EditRates => {
                let rates = self.comparison(self.create.model);
                self.planner.editing = Some(std::array::from_fn(|index| {
                    rates[MODES[index / 5]][COMPONENTS[index % 5].0].to_string()
                }));
            }
            Action::Rate(index, value) => {
                if let Some(rates) = &mut self.planner.editing {
                    rates[index] = value;
                }
            }
            Action::ApplyRates => {
                if let Some(values) = &self.planner.editing {
                    let parsed: Result<Vec<_>> = values
                        .iter()
                        .map(|value| finite_nonnegative(value, 1_000_000_000.0))
                        .collect();
                    match parsed {
                        Ok(values) => {
                            let comparison = MODES
                                .into_iter()
                                .zip(values.chunks_exact(5))
                                .map(|(mode, rates)| {
                                    let rates = COMPONENTS
                                        .into_iter()
                                        .zip(rates)
                                        .map(|((key, _), value)| (key.into(), *value))
                                        .collect();
                                    (mode.into(), rates)
                                })
                                .collect();
                            self.overrides.insert(self.create.model.into(), comparison);
                            self.planner.editing = None;
                            self.notice(self.tr("Applied custom rates for the selected model."));
                        }
                        Err(_) => {
                            self.fail(self.tr("Enter a valid nonnegative number for each rate."))
                        }
                    }
                }
            }
            Action::CancelRates => self.planner.editing = None,
            Action::ResetRates => {
                self.overrides.remove(self.create.model);
                self.planner.editing = None;
            }
            Action::Refresh => {
                let api = self.api.clone();
                return self.start(self.tr("Refreshing published rates…"), async move {
                    let catalog = api
                        .refresh_prices()
                        .await
                        .context("The existing rates are still in use")?;
                    Ok(Outcome::Prices(catalog))
                });
            }
        }
        Task::none()
    }

    pub(super) fn cancel_rate_edit(&mut self) {
        self.planner.editing = None;
    }
}

fn table_row(values: [String; 6]) -> Row<'static, Message> {
    let mut row = Row::new().spacing(8).width(Fill);
    for (index, value) in values.into_iter().enumerate() {
        row = row.push(text(value).size(14).line_height(1.3).width(if index == 0 {
            FillPortion(2)
        } else {
            FillPortion(1)
        }));
    }
    row
}

fn finite_nonnegative(value: &str, max: f64) -> Result<f64> {
    let value: f64 = value.trim().replace(',', ".").parse()?;
    ensure!(
        value.is_finite() && (0.0..=max).contains(&value),
        "Invalid number"
    );
    Ok(value)
}
