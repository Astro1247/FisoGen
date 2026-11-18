mod create;
mod design;
mod gallery;
mod jobs;
mod planner;

use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use iced::widget::{
    Column, Row, button, column, container, image, pick_list, row, rule, scrollable, text,
    text_input,
};
use iced::{
    Alignment, Element, Fill, Length, Subscription, Task, Theme, color, theme::Palette, window,
};

use crate::{
    api::{Api, DraftEntry},
    i18n::{self, Language},
    pricing::{self, Catalog, Comparison},
    storage::{Job, Preferences, Record, Store, now},
};

const APP: &str = "FisoGen";
use design::{ACCENT, ERROR, MINT, MUTED};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Tab {
    Create,
    Gallery,
    Planner,
    Jobs,
    Settings,
}

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Self::Create => "Create / Edit",
            Self::Gallery => "Gallery & History",
            Self::Planner => "Prices & Planner",
            Self::Jobs => "Batch Jobs",
            Self::Settings => "Settings",
        }
    }
}

#[derive(Clone, Debug)]
enum Message {
    Navigate(Tab),
    SetPro(bool),
    SetLanguage(Language),
    ApiKey(String),
    Open(&'static str),
    OpenOutput,
    OpenData,
    Create(create::Action),
    Gallery(gallery::Action),
    Planner(planner::Action),
    Jobs(jobs::Action),
    Done(std::result::Result<Outcome, String>),
    Preview(PathBuf, u32, Option<image::Handle>),
    Close(window::Id),
}

#[derive(Clone, Debug)]
enum Outcome {
    Generated(Box<Record>),
    Drafted(Vec<DraftEntry>),
    Submitted(String),
    Collected(usize),
    Attached,
    Prices(Catalog),
    References(Vec<PathBuf>),
    Exported(PathBuf),
}

pub fn run(store: Store, api: Api) -> iced::Result {
    iced::application(
        move || Studio::new(store.clone(), api.clone()),
        Studio::update,
        Studio::view,
    )
    .title(APP)
    .theme(theme)
    .default_font(design::UI_FONT)
    .subscription(Studio::subscription)
    .window(window::Settings {
        size: iced::Size::new(1180.0, 810.0),
        min_size: Some(iced::Size::new(790.0, 570.0)),
        exit_on_close_request: false,
        position: window::Position::Centered,
        icon: design::window_icon(),
        ..window::Settings::default()
    })
    .run()
}

fn theme(_: &Studio) -> Theme {
    Theme::custom(
        APP.to_owned(),
        Palette {
            background: design::WINDOW,
            text: design::INK,
            primary: ACCENT,
            success: MINT,
            warning: color!(0xffce90),
            danger: color!(0xf78c99),
        },
    )
}

struct Studio {
    store: Store,
    api: Api,
    pro: bool,
    language: Language,
    tab: Tab,
    key: String,
    catalog: Catalog,
    checked: String,
    overrides: BTreeMap<String, Comparison>,
    busy: bool,
    status: String,
    error: bool,
    history: Vec<Record>,
    jobs: Vec<Job>,
    previews: HashMap<(PathBuf, u32), Option<image::Handle>>,
    create: create::Create,
    gallery: gallery::Gallery,
    planner: planner::Planner,
    batch: jobs::Jobs,
}

impl Studio {
    fn new(store: Store, api: Api) -> Self {
        let preferences = store.read_preferences();
        let (pro, language) = preferences
            .as_ref()
            .map(|p| (p.pro, i18n::language(&p.language)))
            .unwrap_or((false, i18n::LANGUAGES[0]));
        let mut state = Self {
            store,
            api,
            pro,
            language,
            tab: Tab::Create,
            key: std::env::var("OPENAI_API_KEY").unwrap_or_default(),
            catalog: pricing::default_catalog(),
            checked: pricing::VERIFIED.into(),
            overrides: BTreeMap::new(),
            busy: false,
            status: i18n::translate(language, "Images save automatically to the outputs folder.")
                .into(),
            error: false,
            history: Vec::new(),
            jobs: Vec::new(),
            previews: HashMap::new(),
            create: create::Create::default(),
            gallery: gallery::Gallery::default(),
            planner: planner::Planner::default(),
            batch: jobs::Jobs::default(),
        };
        state.reload();
        if let Err(error) = preferences {
            state.fail(format!(
                "{} {error:#}",
                state.tr("Could not read settings.")
            ));
        }
        state
    }

    fn subscription(&self) -> Subscription<Message> {
        window::close_requests().map(Message::Close)
    }

    fn view(&self) -> Element<'_, Message> {
        let mut nav = Row::new().spacing(6);
        for tab in [
            Tab::Create,
            Tab::Gallery,
            Tab::Planner,
            Tab::Jobs,
            Tab::Settings,
        ] {
            if !self.pro && matches!(tab, Tab::Planner | Tab::Jobs) {
                continue;
            }
            nav = nav.push(
                button(
                    text(self.tr(tab.label()))
                        .size(15)
                        .font(design::HEADING_FONT),
                )
                .padding([9, 11])
                .style(if tab == self.tab {
                    design::primary
                } else {
                    design::quiet
                })
                .on_press(Message::Navigate(tab)),
            );
        }
        let modes = row![
            button(self.tr("Simple"))
                .style(if self.pro {
                    design::quiet
                } else {
                    design::secondary
                })
                .on_press(Message::SetPro(false)),
            button(self.tr("Pro"))
                .style(if self.pro {
                    design::primary
                } else {
                    design::quiet
                })
                .on_press(Message::SetPro(true)),
        ]
        .spacing(4);
        let wordmark = row![
            image(design::mark()).width(34).height(34),
            text(APP).size(25).font(design::HEADING_FONT)
        ]
        .spacing(11)
        .align_y(Alignment::Center);
        let header = column![
            row![
                wordmark,
                iced::widget::space::horizontal(),
                button(self.tr("Open outputs"))
                    .style(design::secondary)
                    .on_press(Message::OpenOutput),
                modes
            ]
            .spacing(13)
            .width(Fill)
            .align_y(Alignment::Center),
            nav,
        ]
        .spacing(12);
        let page = match self.tab {
            Tab::Create => self.view_create(),
            Tab::Gallery => self.view_gallery(),
            Tab::Planner => self.view_planner(),
            Tab::Jobs => self.view_jobs(),
            Tab::Settings => self.view_settings(),
        };
        let footer = row![
            text(self.tr(if self.busy {
                "Working"
            } else if self.error {
                "Action needed"
            } else {
                "Ready"
            }))
            .size(14)
            .font(design::HEADING_FONT)
            .color(if self.error {
                ERROR
            } else if self.busy {
                ACCENT
            } else {
                MINT
            }),
            text(&self.status).size(14).color(MUTED)
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        column![
            container(header)
                .padding([17, 22])
                .width(Fill)
                .style(design::header),
            container(page).padding([16, 22]).width(Fill).height(Fill),
            rule::horizontal(1),
            container(footer).width(Fill).padding([9, 22])
        ]
        .height(Fill)
        .into()
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Navigate(tab) => {
                self.tab = tab;
                if tab == Tab::Gallery {
                    return self.gallery_previews();
                }
            }
            Message::SetPro(pro) => {
                if self.pro != pro {
                    self.pro = pro;
                    self.create.confirm = false;
                    if !pro && matches!(self.tab, Tab::Planner | Tab::Jobs) {
                        self.tab = Tab::Create;
                    }
                    self.save_preferences();
                    return self.gallery_previews();
                }
            }
            Message::SetLanguage(language) => {
                self.language = language;
                self.save_preferences();
            }
            Message::ApiKey(key) => self.key = key,
            Message::Open(url) => self.report(open::that_detached(url).map_err(Into::into)),
            Message::OpenOutput => {
                self.report(open::that_detached(&self.store.output).map_err(Into::into))
            }
            Message::OpenData => {
                self.report(open::that_detached(&self.store.root).map_err(Into::into))
            }
            Message::Create(action) => return self.update_create(action),
            Message::Gallery(action) => return self.update_gallery(action),
            Message::Planner(action) => return self.update_planner(action),
            Message::Jobs(action) => return self.update_jobs(action),
            Message::Done(result) => return self.done(result),
            Message::Preview(path, size, handle) => {
                self.previews.insert((path, size), handle);
            }
            Message::Close(id) => {
                if self.busy {
                    self.status = self
                        .tr("Wait for the current action to finish before closing.")
                        .into();
                } else {
                    return window::close(id);
                }
            }
        }
        Task::none()
    }

    fn view_settings(&self) -> Element<'_, Message> {
        scrollable(column![card(column![title(self.tr("Settings")),
            field(self.tr("Language"), pick_list(i18n::LANGUAGES, Some(self.language), Message::SetLanguage).width(260)),
            label(self.tr("View mode")), row![button(self.tr("Simple")).style(if self.pro { design::secondary } else { design::primary }).on_press(Message::SetPro(false)),
                button(self.tr("Pro")).style(if self.pro { design::primary } else { design::secondary }).on_press(Message::SetPro(true))].spacing(8),
            label(self.tr("Simple offers single-image creation and the gallery. Pro adds Batch, advanced controls, and the cost planner.")),
            rule::horizontal(1), title(self.tr("Portable data")), label(self.store.root.display().to_string()),
            label(self.tr("Preferences, history, images, and Batch jobs are stored in this folder.")),
            row![button(self.tr("Open data folder")).style(design::secondary).on_press(Message::OpenData),
                button(self.tr("Open outputs")).style(design::secondary).on_press(Message::OpenOutput)].spacing(8)
        ].spacing(11))]).height(Fill).into()
    }

    fn save_preferences(&mut self) {
        let result = self.store.save_preferences(&Preferences {
            language: self.language.code.into(),
            pro: self.pro,
        });
        if let Err(error) = result {
            self.fail(format!("{} {error:#}", self.tr("Could not save settings.")));
        } else if !self.busy {
            self.notice(self.tr("Preferences saved."));
        }
    }

    fn tr(&self, key: &'static str) -> &'static str {
        i18n::translate(self.language, key)
    }
    fn translate_format(&self, key: &'static str, values: &[(&str, String)]) -> String {
        values
            .iter()
            .fold(self.tr(key).to_owned(), |text, (name, value)| {
                text.replace(&format!("{{{name}}}"), value)
            })
    }
    fn tr_component<'a>(&self, value: &'a str) -> &'a str {
        pricing::COMPONENTS
            .iter()
            .find_map(|(_, name)| (*name == value).then(|| self.tr(name)))
            .unwrap_or(value)
    }
    fn money(&self, value: Option<f64>) -> String {
        value.map_or_else(
            || self.tr("Unavailable").into(),
            |value| format!("${value:.6}"),
        )
    }

    fn choice<T: AsRef<str>>(&self, value: T) -> Choice<T> {
        const OPTIONS: [&str; 15] = [
            "All",
            "Standard",
            "Batch",
            "Custom",
            "auto",
            "low",
            "medium",
            "high",
            "xhigh",
            "max",
            "transparent",
            "opaque",
            "png",
            "webp",
            "jpeg",
        ];
        let raw = value.as_ref();
        let localized = OPTIONS
            .into_iter()
            .find(|option| *option == raw)
            .map(|option| self.tr(option))
            .unwrap_or(raw);
        Choice {
            label: localized.to_owned(),
            value,
        }
    }

    fn start(
        &mut self,
        status: &str,
        operation: impl Future<Output = Result<Outcome>> + Send + 'static,
    ) -> Task<Message> {
        if self.busy {
            return Task::none();
        }
        self.busy = true;
        self.notice(status);
        Task::perform(operation, |result| {
            Message::Done(result.map_err(|error| format!("{error:#}")))
        })
    }

    fn done(&mut self, result: std::result::Result<Outcome, String>) -> Task<Message> {
        self.busy = false;
        match result {
            Ok(Outcome::Generated(record)) => {
                self.notice(self.translate_format(
                    "Saved {saved} of {returned} returned images.",
                    &[
                        ("saved", record.files.len().to_string()),
                        ("returned", record.image_count().to_string()),
                    ],
                ));
                if !record.output_errors.is_empty() {
                    self.fail(record.output_errors.join(" "));
                }
                self.create.latest = Some(*record);
                self.create.image_index = 0;
                self.reload();
                return Task::batch([self.create_preview(), self.gallery_previews()]);
            }
            Ok(Outcome::Drafted(entries)) => {
                self.notice(self.translate_format(
                    "Added {count} requests to the draft.",
                    &[("count", entries.len().to_string())],
                ));
                self.create.draft.extend(entries);
            }
            Ok(Outcome::Submitted(id)) => {
                self.create.draft.clear();
                self.create.draft_selected = 0;
                if self.pro {
                    self.tab = Tab::Jobs;
                }
                self.select_latest_job();
                self.notice(self.translate_format(
                    "Submitted {id}. Refresh the job to collect results.",
                    &[("id", id)],
                ));
                self.reload();
            }
            Ok(Outcome::Collected(count)) => {
                self.notice(self.translate_format(
                    "Imported {count} new results. View images and costs in Gallery & History.",
                    &[("count", count.to_string())],
                ));
                self.reload();
                return self.gallery_previews();
            }
            Ok(Outcome::Attached) => {
                self.notice(self.tr("Attached the verified Batch ID."));
                self.batch.recovery.clear();
                self.reload();
            }
            Ok(Outcome::Prices(catalog)) => {
                self.catalog = catalog;
                self.checked = now();
                self.notice(self.tr("Updated the published rates for this session."));
            }
            Ok(Outcome::References(references)) => {
                if !references.is_empty() {
                    self.create.references = references;
                    self.create.confirm = false;
                    self.notice(
                        self.tr("Reference images selected. The request will use image editing."),
                    );
                } else {
                    self.notice(self.tr("Ready"));
                }
            }
            Ok(Outcome::Exported(path)) => self.notice(if path.as_os_str().is_empty() {
                self.tr("Ready").into()
            } else {
                self.translate_format("Saved {path}", &[("path", path.display().to_string())])
            }),
            Err(error) => {
                self.reload();
                self.fail(error);
            }
        }
        Task::none()
    }

    fn reload(&mut self) {
        match self.store.read_history() {
            Ok((history, skipped)) => {
                self.preserve_gallery_selection(&history);
                self.history = history;
                if skipped > 0 {
                    self.fail(self.translate_format(
                        "Skipped {count} unreadable history rows.",
                        &[("count", skipped.to_string())],
                    ));
                }
            }
            Err(error) => self.fail(format!("{} {error:#}", self.tr("Could not read history."))),
        }
        match self.store.read_jobs() {
            Ok(jobs) => self.jobs = jobs,
            Err(error) => self.fail(format!(
                "{} {error:#}",
                self.tr("Could not read Batch jobs.")
            )),
        }
    }

    fn notice(&mut self, message: impl Into<String>) {
        self.status = message.into();
        self.error = false;
    }
    fn fail(&mut self, message: impl Into<String>) {
        self.status = message.into();
        self.error = true;
    }
    fn report(&mut self, result: Result<()>) {
        if let Err(error) = result {
            self.fail(format!("{error:#}"));
        }
    }

    fn comparison(&self, model: &str) -> &Comparison {
        self.overrides.get(model).unwrap_or(&self.catalog[model])
    }

    fn file_task(
        &mut self,
        name: &str,
        label: &str,
        extensions: &[&str],
        write: impl FnOnce(&Path) -> Result<()> + Send + 'static,
    ) -> Task<Message> {
        let dialog = rfd::AsyncFileDialog::new()
            .set_directory(&self.store.root)
            .set_file_name(name)
            .add_filter(label, extensions);
        self.start(self.tr("Choose where to save the file."), async move {
            let Some(file) = dialog.save_file().await else {
                return Ok(Outcome::Exported(PathBuf::new()));
            };
            let path = file.path().to_owned();
            tokio::task::spawn_blocking(move || {
                write(&path).with_context(|| format!("Could not save {}", path.display()))?;
                Ok(Outcome::Exported(path))
            })
            .await?
        })
    }

    fn previews(&mut self, paths: impl IntoIterator<Item = (PathBuf, u32)>) -> Task<Message> {
        if self.previews.len() > 160 {
            self.previews.clear();
        }
        let mut tasks = Vec::new();
        for (path, size) in paths {
            let entry = (path.clone(), size);
            if self.previews.contains_key(&entry) {
                continue;
            }
            self.previews.insert(entry, None);
            tasks.push(Task::perform(
                async move {
                    let read_path = path.clone();
                    let preview = tokio::task::spawn_blocking(move || preview(&read_path, size))
                        .await
                        .ok()
                        .flatten();
                    (path, size, preview)
                },
                |(path, size, preview)| Message::Preview(path, size, preview),
            ));
        }
        Task::batch(tasks)
    }

    fn preview_widget(&self, file: Option<&str>, size: u32, height: f32) -> Element<'_, Message> {
        let path = file.and_then(|file| self.store.image_path(file));
        if let Some(handle) = path
            .and_then(|path| self.previews.get(&(path, size)))
            .and_then(Option::as_ref)
        {
            container(
                image(handle.clone())
                    .width(Fill)
                    .height(Fill)
                    .content_fit(iced::ContentFit::Contain),
            )
            .width(Fill)
            .height(height)
            .style(design::canvas)
            .into()
        } else {
            let caption = self.tr(if file.is_some() {
                "Preview unavailable"
            } else {
                "No image yet"
            });
            if height > 220.0 && file.is_none() {
                container(
                    column![
                        image(design::mark()).width(64).height(64),
                        text(caption)
                            .size(16)
                            .font(design::HEADING_FONT)
                            .color(MUTED)
                    ]
                    .spacing(13)
                    .align_x(Alignment::Center),
                )
                .center_x(Fill)
                .center_y(Length::Fixed(height))
                .style(design::empty_canvas)
                .into()
            } else {
                container(text(caption).size(14).color(MUTED))
                    .center_x(Fill)
                    .center_y(Length::Fixed(height))
                    .style(design::canvas)
                    .into()
            }
        }
    }

    fn key_input(&self) -> Element<'_, Message> {
        column![
            label(self.tr("OpenAI API key")),
            text_input(self.tr("Enter an API key"), &self.key)
                .secure(true)
                .on_input(Message::ApiKey)
                .style(design::input)
                .padding(9)
        ]
        .spacing(5)
        .into()
    }
}

fn preview(path: &Path, size: u32) -> Option<image::Handle> {
    let picture = ::image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?;
    let scaled = picture.thumbnail(size, size).into_rgba8();
    Some(image::Handle::from_rgba(
        scaled.width(),
        scaled.height(),
        scaled.into_raw(),
    ))
}

fn card<'a>(content: impl Into<Element<'a, Message>>) -> container::Container<'a, Message> {
    container(content)
        .padding(18)
        .width(Fill)
        .style(design::surface)
}

fn label<'a>(value: impl Into<String>) -> iced::widget::Text<'a> {
    text(value.into()).size(14).line_height(1.35).color(MUTED)
}
fn title<'a>(value: impl Into<String>) -> iced::widget::Text<'a> {
    text(value.into())
        .size(22)
        .font(design::HEADING_FONT)
        .line_height(1.3)
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Choice<T> {
    value: T,
    label: String,
}

impl<T> std::fmt::Display for Choice<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

fn field<'a>(name: &str, widget: impl Into<Element<'a, Message>>) -> Column<'a, Message> {
    column![label(name), widget.into()].spacing(5).width(Fill)
}

fn tab_link(text: &str, url: &'static str) -> button::Button<'static, Message> {
    button(iced::widget::text(text.to_owned()).size(13))
        .style(design::quiet)
        .on_press(Message::Open(url))
}

fn asset_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}
