use std::sync::OnceLock;

use iced::widget::{button, container, image, text_editor, text_input};
use iced::{Background, Color, Degrees, Font, Theme, border, color, font, gradient, window};

pub const WINDOW: Color = color!(0x131721);
pub const PANEL: Color = color!(0x1c2130);
pub const EDGE: Color = color!(0x383f54);
pub const INK: Color = color!(0xe8ebf3);
pub const MUTED: Color = color!(0xb0b8cc);
pub const ACCENT: Color = color!(0xc4b6ff);
pub const MINT: Color = color!(0x8adbc6);
pub const ERROR: Color = color!(0xffa4ad);
pub const UI_FONT: Font = Font::with_name("Segoe UI");
pub const HEADING_FONT: Font = Font {
    weight: font::Weight::Semibold,
    ..UI_FONT
};
pub const MONO_FONT: Font = Font::with_name("Consolas");
const WELL: Color = color!(0x141925);
const SELECT: Color = color!(0x4b4070);

pub fn surface(_: &Theme) -> container::Style {
    container::Style::default()
        .background(PANEL)
        .border(border::rounded(13).width(1).color(EDGE))
}

pub fn header(_: &Theme) -> container::Style {
    let gradient = gradient::Linear::new(Degrees(110.0))
        .add_stop(0.0, color!(0x26243b))
        .add_stop(1.0, color!(0x1b202e));
    container::Style::default().background(gradient)
}

pub fn canvas(_: &Theme) -> container::Style {
    container::Style::default()
        .background(WELL)
        .border(border::rounded(10).width(1).color(EDGE))
}

pub fn empty_canvas(_: &Theme) -> container::Style {
    let gradient = gradient::Linear::new(Degrees(142.0))
        .add_stop(0.0, color!(0x26253a))
        .add_stop(1.0, color!(0x141b28));
    container::Style::default()
        .background(gradient)
        .border(border::rounded(11).width(1).color(EDGE))
}

pub fn primary(_: &Theme, status: button::Status) -> button::Style {
    let (background, text) = match status {
        button::Status::Active => (ACCENT, color!(0x221939)),
        button::Status::Hovered => (color!(0xd6ccff), color!(0x221939)),
        button::Status::Pressed => (color!(0xb3a1f5), color!(0x221939)),
        button::Status::Disabled => (color!(0x343044), color!(0x9691a5)),
    };
    button_style(Some(background.into()), text, background)
}

pub fn secondary(_: &Theme, status: button::Status) -> button::Style {
    let (background, text) = match status {
        button::Status::Active => (color!(0x2a3043), INK),
        button::Status::Hovered => (color!(0x3a435c), INK),
        button::Status::Pressed => (color!(0x242a3d), INK),
        button::Status::Disabled => (color!(0x202536), color!(0x888fa4)),
    };
    button_style(Some(background.into()), text, EDGE)
}

pub fn quiet(_: &Theme, status: button::Status) -> button::Style {
    let (background, text) = match status {
        button::Status::Active => (None, MUTED),
        button::Status::Hovered => (Some(color!(0x30374c).into()), INK),
        button::Status::Pressed => (Some(color!(0x252b3e).into()), INK),
        button::Status::Disabled => (None, color!(0x888fa4)),
    };
    button_style(background, text, Color::TRANSPARENT)
}

pub fn selected(_: &Theme, status: button::Status) -> button::Style {
    let background = if status == button::Status::Hovered {
        color!(0x3c3552)
    } else {
        color!(0x302a43)
    };
    button_style(Some(background.into()), INK, color!(0x665988))
}

fn button_style(background: Option<Background>, text: Color, edge: Color) -> button::Style {
    button::Style {
        background,
        text_color: text,
        border: border::rounded(8).width(1).color(edge),
        ..button::Style::default()
    }
}

pub fn input(_: &Theme, status: text_input::Status) -> text_input::Style {
    use text_input::Status;
    text_input::Style {
        background: WELL.into(),
        border: input_border(
            matches!(status, Status::Focused { .. }),
            status == Status::Hovered,
        ),
        icon: MUTED,
        placeholder: color!(0x9ca5bb),
        value: INK,
        selection: SELECT,
    }
}

pub fn editor(_: &Theme, status: text_editor::Status) -> text_editor::Style {
    use text_editor::Status;
    text_editor::Style {
        background: WELL.into(),
        border: input_border(
            matches!(status, Status::Focused { .. }),
            status == Status::Hovered,
        ),
        placeholder: color!(0x9ca5bb),
        value: INK,
        selection: SELECT,
    }
}

fn input_border(focused: bool, hovered: bool) -> iced::Border {
    let outline = if focused {
        ACCENT
    } else if hovered {
        color!(0x65708a)
    } else {
        EDGE
    };
    border::rounded(8).width(1).color(outline)
}

pub fn mark() -> image::Handle {
    static MARK: OnceLock<image::Handle> = OnceLock::new();
    MARK.get_or_init(|| image::Handle::from_rgba(128, 128, mark_pixels(128)))
        .clone()
}

pub fn window_icon() -> Option<window::Icon> {
    window::icon::from_rgba(mark_pixels(64), 64, 64).ok()
}

// The app mark is a rounded color swatch with an image outline. Sampling a
// larger source keeps the same mark readable in both the header and title bar.
fn mark_pixels(size: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    let line = |x: f32, y: f32, a: (f32, f32), b: (f32, f32)| {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let along = (((x - a.0) * dx + (y - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        (x - (a.0 + along * dx)).hypot(y - (a.1 + along * dy))
    };
    for row in 0..size {
        for col in 0..size {
            let x = (col as f32 + 0.5) * 64.0 / size as f32;
            let y = (row as f32 + 0.5) * 64.0 / size as f32;
            let corner = (x - x.clamp(16.0, 48.0)).hypot(y - y.clamp(16.0, 48.0));
            let alpha = ((14.5 - corner) * size as f32 / 64.0 + 0.5).clamp(0.0, 1.0);
            let frame = (x - x.clamp(20.0, 44.0)).hypot(y - y.clamp(22.0, 42.0));
            let ridge = [
                ((15.5, 42.0), (26.0, 31.0)),
                ((26.0, 31.0), (33.0, 38.0)),
                ((33.0, 38.0), (39.0, 33.0)),
                ((39.0, 33.0), (48.0, 42.0)),
            ]
            .into_iter()
            .map(|(a, b)| line(x, y, a, b))
            .fold(f32::INFINITY, f32::min);
            let mark = (frame - 4.0)
                .abs()
                .min(ridge)
                .min((x - 39.0).hypot(y - 26.0) - 1.7);
            let ink = ((1.35 - mark) * size as f32 / 64.0 + 0.5).clamp(0.0, 1.0);
            let mix = ((x + y - 16.0) / 96.0).clamp(0.0, 1.0);
            for (left, right, dark) in [
                (208.0, 110.0, 35.0),
                (179.0, 219.0, 26.0),
                (255.0, 204.0, 54.0),
            ] {
                let tint = left + (right - left) * mix;
                pixels.push((tint + (dark - tint) * ink).round() as u8);
            }
            pixels.push((alpha * 255.0).round() as u8);
        }
    }
    pixels
}
