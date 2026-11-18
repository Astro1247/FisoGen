use std::collections::BTreeMap;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MODELS: [&str; 4] = [
    "gpt-image-2.6-sunburst",
    "gpt-image-2.6-flare",
    "gpt-image-2.5-sunburst",
    "gpt-image-2.5-flare",
];
pub const MODES: [&str; 2] = ["Standard", "Batch"];
pub const COMPONENTS: [(&str, &str); 5] = [
    ("text_input", "Text input"),
    ("cached_text_input", "Cached text input"),
    ("image_input", "Image input"),
    ("cached_image_input", "Cached image input"),
    ("image_output", "Image output"),
];
pub const VERIFIED: &str = "2026-11-18";
pub const PRICING_URL: &str = "https://developers.openai.com/api/docs/pricing#image-generation";
pub const PRICING_MARKDOWN: &str = "https://developers.openai.com/api/docs/pricing.md";
pub const CALCULATOR: &str =
    "https://developers.openai.com/api/docs/guides/image-generation#calculating-costs";
pub const CACHE: &str =
    "https://developers.openai.com/api/docs/guides/image-generation#cached-input-pricing";

pub type Rates = BTreeMap<String, f64>;
pub type Comparison = BTreeMap<String, Rates>;
pub type Catalog = BTreeMap<String, Comparison>;

pub fn default_catalog() -> Catalog {
    let rates = [[5.0, 1.25, 8.0, 2.0, 30.0], [2.5, 0.625, 4.0, 1.0, 15.0]];
    MODELS
        .into_iter()
        .map(|model| {
            let modes = MODES
                .into_iter()
                .zip(rates)
                .map(|(mode, numbers)| {
                    let values = COMPONENTS
                        .into_iter()
                        .zip(numbers)
                        .map(|((key, _), value)| (key.to_owned(), value))
                        .collect();
                    (mode.to_owned(), values)
                })
                .collect();
            (model.to_owned(), modes)
        })
        .collect()
}

pub fn parse_official_prices(markdown: &str) -> Result<Catalog> {
    let mut sections = markdown.lines();
    ensure!(
        sections.any(|line| line.trim_end() == "Images"),
        "Image pricing section was not found."
    );
    let mut catalog = Catalog::new();
    let mut mode = None;
    for line in sections.take_while(|line| line.trim_end() != "Audio") {
        if MODES.contains(&line.trim()) {
            mode = Some(line.trim());
        }
        let Some(mode) = mode else { continue };
        if !line.starts_with('|') {
            continue;
        }
        let cells: Vec<_> = line
            .trim()
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        let [model, modality, input, output, cached] = cells.as_slice() else {
            continue;
        };
        if !MODELS.contains(model) {
            continue;
        }
        let parse = |price: &str| -> Result<f64> {
            let value: f64 = price
                .replace(['$', ','], "")
                .parse()
                .context("Invalid published price")?;
            ensure!(value.is_finite() && value >= 0.0, "Invalid published price");
            Ok(value)
        };
        let rates = catalog
            .entry((*model).to_owned())
            .or_default()
            .entry(mode.to_owned())
            .or_default();
        match *modality {
            "Text" => {
                rates.insert("text_input".into(), parse(input)?);
                rates.insert("cached_text_input".into(), parse(cached)?);
            }
            "Image" => {
                rates.insert("image_input".into(), parse(input)?);
                rates.insert("image_output".into(), parse(output)?);
                rates.insert("cached_image_input".into(), parse(cached)?);
            }
            _ => {}
        }
    }
    for model in MODELS {
        for mode in MODES {
            ensure!(
                catalog
                    .get(model)
                    .and_then(|m| m.get(mode))
                    .is_some_and(|r| r.len() == COMPONENTS.len()),
                "The official page is missing expected rates for {model} ({mode})."
            );
        }
    }
    Ok(catalog)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CostLine {
    pub component: String,
    pub key: String,
    pub tokens: Option<u64>,
    pub rate: Option<f64>,
    pub usd: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Cost {
    pub text: Option<f64>,
    pub image: Option<f64>,
    pub output: Option<f64>,
    pub lines: Vec<CostLine>,
    pub subtotal: Option<f64>,
    pub known_subtotal: f64,
    pub uplift_pct: f64,
    pub uplift: f64,
    pub known_total: f64,
    pub total: Option<f64>,
    pub warning: Option<String>,
}

pub fn estimate(
    usage: &Value,
    rates: &Rates,
    cache_eligible: bool,
    uplift_pct: f64,
) -> Option<Cost> {
    if usage.as_object().is_none_or(|value| value.is_empty()) {
        return None;
    }
    let details = &usage["input_tokens_details"];
    let total_input = usage["input_tokens"].as_u64();
    let mut text = details["text_tokens"].as_u64();
    let mut image = details["image_tokens"].as_u64();
    let mut notes = Vec::new();
    if total_input == Some(0) {
        text = text.or(Some(0));
        image = image.or(Some(0));
    }
    if let (Some(text_count), Some(image_count), Some(total)) = (text, image, total_input)
        && text_count.checked_add(image_count) != Some(total)
    {
        notes.push("Reported input totals do not match the modality split.");
        (text, image) = (None, None);
    }

    let cached = &details["cached_tokens_details"];
    let reported = details["cached_tokens"].as_u64();
    let mut cached_text = cached["text_tokens"].as_u64();
    let mut cached_image = cached["image_tokens"].as_u64();
    if cache_eligible {
        match (cached_text, cached_image, reported) {
            (None, None, None | Some(0)) => {
                (cached_text, cached_image) = (Some(0), Some(0));
                notes.push("No cache hits reported. The estimate uses uncached input rates.");
            }
            (Some(a), Some(b), total) if total.is_none() || a.checked_add(b) == total => {}
            _ => {
                (cached_text, cached_image) = (None, None);
                notes.push("The cached input modality split is unavailable.");
            }
        }
    } else {
        (cached_text, cached_image) = (Some(0), Some(0));
        if reported.unwrap_or_default() > 0 || cached.as_object().is_some_and(|v| !v.is_empty()) {
            notes.push("Cached discounts do not apply to this direct Images API request.");
        }
    }
    if text
        .zip(cached_text)
        .is_some_and(|(all, cached)| cached > all)
        || image
            .zip(cached_image)
            .is_some_and(|(all, cached)| cached > all)
    {
        (cached_text, cached_image) = (None, None);
        notes.push("Reported cached tokens exceed input tokens.");
    }

    let quantities = [
        text.zip(cached_text).map(|(a, b)| a - b),
        cached_text,
        image.zip(cached_image).map(|(a, b)| a - b),
        cached_image,
        usage["output_tokens"].as_u64(),
    ];
    let lines: Vec<_> = COMPONENTS
        .into_iter()
        .zip(quantities)
        .map(|((key, component), tokens)| {
            let rate = rates.get(key).copied();
            let usd = tokens
                .zip(rate)
                .map(|(tokens, rate)| tokens as f64 * rate / 1e6)
                .or_else(|| (tokens == Some(0)).then_some(0.0));
            CostLine {
                component: component.into(),
                key: key.into(),
                tokens,
                rate,
                usd,
            }
        })
        .collect();

    let complete = lines.iter().all(|line| line.usd.is_some());
    let known_subtotal: f64 = lines.iter().filter_map(|line| line.usd).sum();
    let uplift = known_subtotal * uplift_pct / 100.0;
    let known_total = known_subtotal + uplift;
    if !complete {
        notes.push("The request total is unavailable. Known components are shown.");
    }
    let combined = |first: usize, second: usize| Some(lines[first].usd? + lines[second].usd?);
    Some(Cost {
        text: combined(0, 1),
        image: combined(2, 3),
        output: lines[4].usd,
        lines,
        subtotal: complete.then_some(known_subtotal),
        known_subtotal,
        uplift_pct,
        uplift,
        known_total,
        total: complete.then_some(known_total),
        warning: (!notes.is_empty()).then(|| notes.join(" ")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn estimates_reported_and_partial_usage() {
        let catalog = default_catalog();
        let rates = &catalog[MODELS[0]]["Standard"];
        let usage = json!({"input_tokens": 3000, "output_tokens": 3000,
            "input_tokens_details": {"text_tokens": 1000, "image_tokens": 2000}});
        assert!((estimate(&usage, rates, true, 0.0).unwrap().total.unwrap() - 0.111).abs() < 1e-9);
        let partial = estimate(
            &json!({"input_tokens": 3, "output_tokens": 1000}),
            rates,
            false,
            0.0,
        )
        .unwrap();
        assert_eq!(partial.total, None);
        assert_eq!(partial.known_total, 0.03);
        assert_eq!(
            estimate(
                &json!({"input_tokens": 0, "output_tokens": 0}),
                rates,
                false,
                0.0
            )
            .unwrap()
            .total,
            Some(0.0)
        );
    }

    #[test]
    fn rejects_incomplete_price_refresh() {
        let mut page = String::from("Images\n");
        for (mode, output) in [("Standard", 30), ("Batch", 15)] {
            page.push_str(&format!("{mode}\n"));
            for model in MODELS {
                page.push_str(&format!("| {model} | Text | $5 | - | $1.25 |\n| {model} | Image | $8 | ${output} | $2 |\n"));
            }
        }
        assert_eq!(
            parse_official_prices(&page).unwrap()[MODELS[3]]["Batch"]["image_output"],
            15.0
        );
        assert!(
            parse_official_prices(&page.lines().take(9).collect::<Vec<_>>().join("\n")).is_err()
        );
    }
}
