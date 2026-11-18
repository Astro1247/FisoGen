use std::{
    collections::HashSet,
    fs, io,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::TryStreamExt;
use reqwest::{Client, Method, RequestBuilder, Response, StatusCode, multipart};
use serde::Serialize;
use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::io::StreamReader;
use uuid::Uuid;

use crate::{
    pricing,
    storage::{Job, Metadata, Record, Store, now},
};

const API: &str = "https://api.openai.com";
const MAX_REQUESTS: usize = 50_000;
const MAX_BATCH_BYTES: usize = 200_000_000;
const MAX_REF_BYTES: u64 = 15_728_600;

#[derive(Clone, Debug)]
pub struct DraftEntry {
    pub id: String,
    pub endpoint: &'static str,
    pub metadata: Metadata,
    pub line: Arc<[u8]>,
}

#[derive(Serialize)]
struct BatchRow<'a> {
    custom_id: &'a str,
    method: &'static str,
    url: &'static str,
    body: &'a Value,
}

fn endpoint(references: &[PathBuf]) -> &'static str {
    if references.is_empty() {
        "/v1/images/generations"
    } else {
        "/v1/images/edits"
    }
}

fn request_body(meta: &Metadata, references: &[PathBuf]) -> Result<Value> {
    let mut body = json!({"model": meta.model, "prompt": meta.prompt, "size": meta.size,
        "quality": meta.quality, "background": meta.background, "output_format": meta.format, "n": meta.requested});
    if let Some(complexity) = &meta.complexity {
        body["complexity"] = json!(complexity);
    }
    ensure!(references.len() <= 16, "Choose up to 16 reference images.");
    let mut images = Vec::new();
    let mut total = 0;
    for path in references {
        let mime = match path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            _ => bail!("Unsupported reference image: {}", path.display()),
        };
        let size = fs::metadata(path)
            .with_context(|| format!("Could not read {}", path.display()))?
            .len();
        ensure!(
            size <= MAX_REF_BYTES,
            "{} exceeds the JSON image reference limit.",
            path.display()
        );
        total += size;
        ensure!(
            total < MAX_BATCH_BYTES as u64 * 3 / 4,
            "The reference images are too large together."
        );
        let bytes = fs::read(path).with_context(|| format!("Could not read {}", path.display()))?;
        ensure!(
            bytes.len() as u64 <= MAX_REF_BYTES,
            "{} exceeds the JSON image reference limit.",
            path.display()
        );
        images.push(json!({"image_url": format!("data:{mime};base64,{}", STANDARD.encode(bytes))}));
    }
    if !images.is_empty() {
        body["images"] = Value::Array(images);
    }
    Ok(body)
}

pub fn draft_entries(
    meta: Metadata,
    references: &[PathBuf],
    current: &[DraftEntry],
) -> Result<Vec<DraftEntry>> {
    let mut prompts = Vec::new();
    let mut pending = String::new();
    for line in meta.prompt.lines() {
        if line.trim() == "---" {
            if !pending.trim().is_empty() {
                prompts.push(pending.trim().to_owned());
            }
            pending.clear();
        } else {
            pending.push_str(line);
            pending.push('\n');
        }
    }
    if !pending.trim().is_empty() {
        prompts.push(pending.trim().to_owned());
    }
    ensure!(!prompts.is_empty(), "Enter at least one prompt.");
    ensure!(
        current.len() + prompts.len() <= MAX_REQUESTS,
        "A Batch can contain up to 50,000 requests."
    );
    let endpoint = endpoint(references);
    if let Some(first) = current.first() {
        ensure!(
            first.metadata.model == meta.model,
            "A Batch uses one model. Submit or remove the current draft before switching models."
        );
        ensure!(
            first.endpoint == endpoint,
            "Queue generations and reference-image edits in separate batches."
        );
    }
    let mut size: usize = current.iter().map(|entry| entry.line.len()).sum();
    let mut body = request_body(&meta, references)?;
    let mut additions = Vec::new();
    for prompt in prompts {
        body["prompt"] = json!(prompt);
        let id = format!("image-{}", Uuid::new_v4().simple());
        let mut line = serde_json::to_vec(&BatchRow {
            custom_id: &id,
            method: "POST",
            url: endpoint,
            body: &body,
        })?;
        line.push(b'\n');
        size += line.len();
        ensure!(
            size <= MAX_BATCH_BYTES,
            "Batch input exceeds 200 MB. Reduce the number of prompts or reference sizes."
        );
        let metadata = Metadata {
            prompt,
            ..meta.clone()
        };
        additions.push(DraftEntry {
            id,
            endpoint,
            metadata,
            line: line.into(),
        });
    }
    Ok(additions)
}

#[derive(Clone, Debug)]
pub struct Api(Client);

impl Api {
    pub fn new() -> Result<Self> {
        Ok(Self(
            Client::builder()
                .connect_timeout(Duration::from_secs(20))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
        ))
    }

    fn request(&self, method: Method, path: &str, key: &str, seconds: u64) -> RequestBuilder {
        self.0
            .request(method, format!("{API}{path}"))
            .bearer_auth(key)
            .timeout(Duration::from_secs(seconds))
    }

    pub async fn refresh_prices(&self) -> Result<pricing::Catalog> {
        let response = self
            .0
            .get(pricing::PRICING_MARKDOWN)
            .timeout(Duration::from_secs(20))
            .send()
            .await?;
        let response = check_response(response, "").await?;
        pricing::parse_official_prices(&response.text().await?)
    }

    pub async fn generate(
        &self,
        store: &Store,
        key: &str,
        metadata: Metadata,
        refs: Vec<PathBuf>,
    ) -> Result<Record> {
        let start = Instant::now();
        let response = self
            .request(Method::POST, endpoint(&refs), key, 600)
            .json(&request_body(&metadata, &refs)?)
            .send()
            .await?;
        let response = check_response(response, key).await?;
        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|header| header.to_str().ok())
            .map(str::to_owned);
        let body: Value = response.json().await?;
        let details = json!({"request_id": request_id, "elapsed_s": start.elapsed().as_secs_f64()});
        let record = make_record(store, &body, metadata, Uuid::new_v4(), Value::Null, details)?;
        store.append_history(&record)?;
        Ok(record)
    }

    pub async fn submit(&self, store: &Store, key: &str, draft: &[DraftEntry]) -> Result<String> {
        let first = draft
            .first()
            .context("Add requests to the Batch draft first.")?;
        let mut payload = Vec::with_capacity(draft.iter().map(|entry| entry.line.len()).sum());
        for entry in draft {
            payload.extend_from_slice(&entry.line);
        }
        let mut job = Job {
            local_id: Uuid::new_v4().simple().to_string(),
            time: now(),
            status: "uploading".into(),
            endpoint: first.endpoint.into(),
            metadata: first.metadata.clone(),
            requests: draft
                .iter()
                .map(|entry| (entry.id.clone(), entry.metadata.prompt.clone()))
                .collect(),
            request_metadata: draft
                .iter()
                .map(|entry| (entry.id.clone(), entry.metadata.clone()))
                .collect(),
            ..Job::default()
        };
        store.save_job(&job)?;
        match self.submit_job(store, key, payload, &mut job).await {
            Ok(id) => Ok(id),
            Err(error) => {
                if job.batch_id.is_some() {
                    return Err(error);
                }
                let uncertain = job.status == "submitting"
                    && !error
                        .downcast_ref::<HttpError>()
                        .is_some_and(|e| e.status.is_client_error());
                job.status = if uncertain {
                    "submission_unknown"
                } else {
                    "submission_failed"
                }
                .into();
                job.error = Some(redact(&format!("{error:#}"), key));
                store
                    .save_job(&job)
                    .context("Could not update the local Batch job after a failed submission")?;
                if uncertain {
                    bail!(
                        "Submission outcome is unknown. Check Batch jobs and the OpenAI Batch dashboard before submitting again. {error:#}"
                    );
                }
                Err(error)
            }
        }
    }

    async fn submit_job(
        &self,
        store: &Store,
        key: &str,
        payload: Vec<u8>,
        job: &mut Job,
    ) -> Result<String> {
        let file = multipart::Part::bytes(payload)
            .file_name("images.jsonl")
            .mime_str("application/jsonl")?;
        let form = multipart::Form::new()
            .text("purpose", "batch")
            .part("file", file);
        let uploaded = self
            .request(Method::POST, "/v1/files", key, 120)
            .multipart(form)
            .send()
            .await?;
        let uploaded: Value = check_response(uploaded, key).await?.json().await?;
        let file_id = uploaded["id"]
            .as_str()
            .context("OpenAI did not return an input file ID.")?;
        job.input_file_id = Some(file_id.into());
        job.status = "submitting".into();
        store.save_job(job)?;
        let response = self
            .request(Method::POST, "/v1/batches", key, 120)
            .json(&json!({
                "input_file_id": file_id, "endpoint": job.endpoint, "completion_window": "24h",
                "metadata": {"studio_job": job.local_id}
            }))
            .send()
            .await?;
        let batch: Value = check_response(response, key).await?.json().await?;
        let batch_id = batch["id"]
            .as_str()
            .context("OpenAI did not return a Batch ID.")?
            .to_owned();
        set_batch(job, &batch_id, batch)?;
        store.save_job(job).context("The Batch was submitted, but its response could not be saved. Check the OpenAI dashboard before submitting again.")?;
        Ok(batch_id)
    }

    async fn get_batch(&self, key: &str, id: &str) -> Result<Value> {
        ensure!(valid_id(id, "batch_"), "Enter a valid batch_… ID.");
        let response = self
            .request(Method::GET, &format!("/v1/batches/{id}"), key, 60)
            .send()
            .await?;
        Ok(check_response(response, key).await?.json().await?)
    }

    pub async fn attach(&self, store: &Store, key: &str, mut job: Job, id: &str) -> Result<()> {
        let expected = job
            .input_file_id
            .as_deref()
            .context("There is no saved input file for this job.")?;
        let batch = self.get_batch(key, id).await?;
        ensure!(
            batch["input_file_id"].as_str() == Some(expected),
            "This Batch uses a different input file."
        );
        set_batch(&mut job, id, batch)?;
        store.save_job(&job)
    }

    pub async fn collect(&self, store: &Store, key: &str, mut job: Job) -> Result<usize> {
        let batch_id = job
            .batch_id
            .clone()
            .context("This job does not have a Batch ID yet.")?;
        let batch = self.get_batch(key, &batch_id).await?;
        set_batch(&mut job, &batch_id, batch)?;
        job.details.insert("checked_at".into(), json!(now()));
        store.save_job(&job)?;
        let mut seen: HashSet<_> = store
            .read_history()?
            .0
            .into_iter()
            .map(|record| record.record_id)
            .collect();
        let mut imported = 0;
        for field in ["output_file_id", "error_file_id"] {
            let Some(id) = job.batch[field].as_str().map(str::to_owned) else {
                continue;
            };
            if job.imported_files.contains(&id) {
                continue;
            }
            ensure!(
                valid_id(&id, "file-") || valid_id(&id, "file_"),
                "Invalid Batch result file ID."
            );
            let response = self
                .request(Method::GET, &format!("/v1/files/{id}/content"), key, 120)
                .send()
                .await?;
            let response = check_response(response, key).await?;
            let stream = StreamReader::new(response.bytes_stream().map_err(io::Error::other));
            let mut reader = BufReader::new(stream);
            let mut line = Vec::new();
            while reader.read_until(b'\n', &mut line).await? != 0 {
                if !line.iter().all(u8::is_ascii_whitespace) {
                    let result: Value = serde_json::from_slice(&line)
                        .context("Invalid JSON in the Batch results")?;
                    if import_result(store, &job, &batch_id, &result, &mut seen)? {
                        imported += 1;
                    }
                }
                line.clear();
            }
            job.imported_files.push(id);
            store.save_job(&job)?;
        }
        Ok(imported)
    }
}

fn import_result(
    store: &Store,
    job: &Job,
    batch_id: &str,
    result: &Value,
    seen: &mut HashSet<String>,
) -> Result<bool> {
    let custom_id = result["custom_id"]
        .as_str()
        .context("A Batch result is missing its custom_id.")?;
    let prompt = job
        .requests
        .get(custom_id)
        .with_context(|| format!("Unknown Batch request ID {custom_id}"))?;
    let id = Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("{batch_id}/{custom_id}").as_bytes(),
    );
    let record_id = id.simple().to_string();
    if seen.contains(&record_id) {
        return Ok(false);
    }
    let response = &result["response"];
    let body = &response["body"];
    let mut error = result["error"].clone();
    if error.is_null() {
        error = body["error"].clone();
    }
    if error.is_null()
        && let Some(status) = response["status_code"].as_u64().filter(|n| *n != 200)
    {
        error = json!(format!("HTTP {status}"));
    }
    let metadata = Metadata {
        prompt: prompt.clone(),
        ..job
            .request_metadata
            .get(custom_id)
            .unwrap_or(&job.metadata)
            .clone()
    };
    let details = json!({"batch_id": batch_id, "custom_id": custom_id, "request_id": response["request_id"], "elapsed_s": null});
    let record = make_record(store, body, metadata, id, error, details)?;
    store.append_history(&record)?;
    seen.insert(record_id);
    Ok(true)
}

fn make_record(
    store: &Store,
    body: &Value,
    metadata: Metadata,
    id: Uuid,
    error: Value,
    details: Value,
) -> Result<Record> {
    let record_id = id.simple().to_string();
    let mut files = Vec::new();
    let mut output_errors = Vec::new();
    let data = body["data"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    for (index, image) in data.iter().enumerate() {
        let index = index + 1;
        let Some(encoded) = image["b64_json"].as_str() else {
            output_errors.push(format!("Output {index} did not contain an image."));
            continue;
        };
        match STANDARD.decode(encoded) {
            Ok(bytes) if !bytes.is_empty() => {
                match store.save_image(&record_id, index, &metadata.format, &bytes) {
                    Ok(filename) => files.push(filename),
                    Err(error) => {
                        output_errors.push(format!("Output {index} could not be saved: {error:#}"))
                    }
                }
            }
            _ => output_errors.push(format!("Output {index} contained invalid image data.")),
        }
    }
    let mut details: Map<String, Value> = details.as_object().cloned().unwrap_or_default();
    details.insert("response_id".into(), body["id"].clone());
    let usage = body["usage"].clone();
    let cost = pricing::estimate(
        &usage,
        &metadata.rates,
        metadata.model.starts_with("gpt-image-2.6"),
        metadata.uplift_pct,
    );
    Ok(Record {
        metadata,
        record_id,
        time: now(),
        files,
        returned_images: Some(data.len()),
        usage,
        cost,
        error,
        output_errors,
        details,
    })
}

fn set_batch(job: &mut Job, id: &str, batch: Value) -> Result<()> {
    ensure!(
        valid_id(id, "batch_"),
        "OpenAI supplied an invalid Batch ID."
    );
    job.status = batch["status"]
        .as_str()
        .context("OpenAI did not return a Batch status.")?
        .into();
    job.batch_id = Some(id.into());
    job.batch = batch;
    job.error = None;
    Ok(())
}

fn valid_id(id: &str, prefix: &str) -> bool {
    id.strip_prefix(prefix).is_some_and(|tail| {
        !tail.is_empty()
            && tail
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    })
}

fn redact(message: &str, key: &str) -> String {
    if key.is_empty() {
        message.into()
    } else {
        message.replace(key, "[redacted]")
    }
}

#[derive(Debug)]
struct HttpError {
    status: StatusCode,
    message: String,
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HTTP {}: {}", self.status, self.message)
    }
}
impl std::error::Error for HttpError {}

async fn check_response(response: Response, key: &str) -> Result<Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let text = response.text().await.unwrap_or_default();
    let json: Value = serde_json::from_str(&text).unwrap_or_default();
    let message = json["error"]["message"].as_str().unwrap_or(&text);
    let message = redact(message, key).chars().take(750).collect();
    Err(HttpError { status, message }.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_preserves_request_settings_and_rejects_mixed_models() {
        let meta = Metadata {
            prompt: "First\n---\nSecond".into(),
            model: pricing::MODELS[0].into(),
            quality: "high".into(),
            ..Metadata::default()
        };
        let draft = draft_entries(meta.clone(), &[], &[]).unwrap();
        assert_eq!(draft.len(), 2);
        assert_eq!(draft[1].metadata.prompt, "Second");
        assert_eq!(
            serde_json::from_slice::<Value>(&draft[1].line).unwrap()["body"]["quality"],
            "high"
        );
        assert!(
            draft_entries(
                Metadata {
                    model: pricing::MODELS[1].into(),
                    ..meta
                },
                &[],
                &draft
            )
            .is_err()
        );
    }

    #[test]
    fn batch_import_is_idempotent_and_uses_saved_metadata() {
        let folder = tempfile::tempdir().unwrap();
        let store = Store::open(folder.path()).unwrap();
        let job = Job {
            requests: [("request".into(), "original prompt".into())].into(),
            request_metadata: [(
                "request".into(),
                Metadata {
                    model: pricing::MODELS[0].into(),
                    format: "png".into(),
                    ..Metadata::default()
                },
            )]
            .into(),
            ..Job::default()
        };
        let result =
            json!({"custom_id": "request", "response": {"body": {"data": [{"b64_json": "YQ=="}]}}});
        let mut seen = HashSet::new();
        assert!(import_result(&store, &job, "batch_example", &result, &mut seen).unwrap());
        assert!(!import_result(&store, &job, "batch_example", &result, &mut seen).unwrap());
        let records = store.read_history().unwrap().0;
        assert_eq!(records[0].metadata.prompt, "original prompt");
        assert_eq!(
            fs::read(store.output.join(&records[0].files[0])).unwrap(),
            b"a"
        );
    }
}
