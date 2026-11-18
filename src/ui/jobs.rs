use iced::widget::{Column, button, column, container, row, scrollable, text, text_input};
use iced::{Alignment, Element, Fill, FillPortion, Task};

use super::{
    ERROR, Message, Outcome, Studio, card, design, field, gallery::short_time, label, tab_link,
    title,
};

#[derive(Clone, Debug)]
pub enum Action {
    Select(usize),
    Refresh,
    Recovery(String),
    Attach,
    Details,
}

#[derive(Default)]
pub struct Jobs {
    selected: usize,
    pub recovery: String,
    details: bool,
}

fn msg(action: Action) -> Message {
    Message::Jobs(action)
}

impl Studio {
    pub(super) fn view_jobs(&self) -> Element<'_, Message> {
        let mut list = column![
            title(self.tr("Batch Jobs")),
            label(self.tr("Refresh a job to collect available images and costs.")),
            row![
                tab_link(
                    self.tr("Open Batch dashboard"),
                    "https://platform.openai.com/batches"
                ),
                tab_link(
                    self.tr("Batch guide"),
                    "https://developers.openai.com/api/docs/guides/batch"
                )
            ]
            .spacing(6),
        ]
        .spacing(10);
        if self.jobs.is_empty() {
            list = list.push(label(self.tr(
                "No Batch jobs yet. Choose Batch on the Create / Edit tab to start a draft.",
            )));
        }
        let rows = Column::with_children(self.jobs.iter().enumerate().map(|(index, job)| {
            button(
                column![
                    text(job.batch_id.as_deref().unwrap_or(&job.local_id)).size(14),
                    label(format!(
                        "{}  ·  {}  ·  {} {}",
                        short_time(&job.time),
                        self.job_status(&job.status),
                        job.requests.len(),
                        self.tr("requests")
                    ))
                ]
                .spacing(4),
            )
            .width(Fill)
            .padding(8)
            .style(if index == self.batch.selected {
                design::secondary
            } else {
                design::quiet
            })
            .on_press(msg(Action::Select(index)))
            .into()
        }))
        .spacing(4);
        list = list.push(scrollable(rows).height(Fill));
        let mut details = column![title(self.tr("Job details")), self.key_input()].spacing(11);
        if let Some(job) = self.jobs.get(self.batch.selected) {
            let has_key = !self.busy && !self.key.trim().is_empty();
            details = details
                .push(text(self.job_status(&job.status)).size(19))
                .push(label(format!(
                    "{}  ·  {}  ·  {} {}",
                    short_time(&job.time),
                    job.metadata.model,
                    job.requests.len(),
                    self.tr("requests")
                )));
            if let Some(error) = &job.error {
                details = details.push(text(error).size(14).color(ERROR));
            }
            if job.batch_id.is_some() {
                let counts = &job.batch["request_counts"];
                if counts.is_object() {
                    details = details.push(label(format!(
                        "{}: {} / {}  ·  {}: {}",
                        self.tr("Completed"),
                        counts["completed"],
                        counts["total"],
                        self.tr("Failed"),
                        counts["failed"]
                    )));
                }
                details = details.push(
                    button(self.tr("Refresh status & collect results"))
                        .on_press_maybe(has_key.then_some(msg(Action::Refresh))),
                );
                if !job.batch["errors"].is_null() {
                    details =
                        details.push(text(job.batch["errors"].to_string()).size(14).color(ERROR));
                }
            } else if matches!(job.status.as_str(), "submission_unknown" | "submitting")
                && job.input_file_id.is_some()
            {
                details = details.push(label(self.tr("Submission outcome is unknown. Check the OpenAI dashboard before submitting again.")))
                    .push(row![field(self.tr("Recovered Batch ID"), text_input("batch_…", &self.batch.recovery).on_input(|value| msg(Action::Recovery(value)))),
                        button(self.tr("Attach Batch ID")).style(design::secondary)
                            .on_press_maybe((has_key && !self.batch.recovery.trim().is_empty()).then_some(msg(Action::Attach)))].spacing(9).align_y(Alignment::End));
            }
            details = details
                .push(label(self.tr(
                    "Collect results within 30 days. OpenAI Batch result files expire by default.",
                )))
                .push(
                    button(if self.batch.details {
                        self.tr("Hide raw details")
                    } else {
                        self.tr("Show raw details")
                    })
                    .style(design::secondary)
                    .on_press(msg(Action::Details)),
                );
            if self.batch.details {
                details = details.push(
                    text(serde_json::to_string_pretty(job).unwrap_or_default())
                        .font(design::MONO_FONT)
                        .size(14),
                );
            }
        } else {
            details = details.push(label(
                self.tr("Select a job to view its status, request counts, and saved rates."),
            ));
        }
        row![
            container(card(list).height(Fill)).width(FillPortion(5)),
            container(scrollable(card(details)).height(Fill)).width(FillPortion(6))
        ]
        .spacing(14)
        .height(Fill)
        .into()
    }

    fn job_status<'a>(&self, status: &'a str) -> &'a str {
        match status {
            "uploading" => self.tr("Uploading"),
            "submitting" => self.tr("Submitting"),
            "validating" => self.tr("Validating"),
            "in_progress" => self.tr("In progress"),
            "finalizing" => self.tr("Finalizing"),
            "completed" => self.tr("Completed"),
            "failed" | "submission_failed" => self.tr("Failed"),
            "expired" => self.tr("Expired"),
            "cancelling" => self.tr("Cancelling"),
            "cancelled" => self.tr("Cancelled"),
            "submission_unknown" => self.tr("Submission unknown"),
            _ => status,
        }
    }

    pub(super) fn update_jobs(&mut self, action: Action) -> Task<Message> {
        match action {
            Action::Select(index) => {
                self.batch.selected = index;
                self.batch.recovery.clear();
                self.batch.details = false;
            }
            Action::Recovery(value) => self.batch.recovery = value,
            Action::Details => self.batch.details = !self.batch.details,
            Action::Refresh | Action::Attach => {
                let Some(job) = self.jobs.get(self.batch.selected).cloned() else {
                    return Task::none();
                };
                if self.key.trim().is_empty() || self.busy {
                    return Task::none();
                }
                let (api, store, key) = (
                    self.api.clone(),
                    self.store.clone(),
                    self.key.trim().to_owned(),
                );
                if matches!(action, Action::Refresh) {
                    return self.start(self.tr("Checking Batch status and importing available images…"), async move {
                        Ok(Outcome::Collected(api.collect(&store, &key, job).await?))
                    });
                }
                let recovery = self.batch.recovery.trim().to_owned();
                return self.start(self.tr("Verifying the Batch ID…"), async move {
                    api.attach(&store, &key, job, &recovery).await?;
                    Ok(Outcome::Attached)
                });
            }
        }
        Task::none()
    }

    pub(super) fn select_latest_job(&mut self) {
        self.batch.selected = 0;
        self.batch.details = false;
    }
}
