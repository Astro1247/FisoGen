use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, ensure};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tempfile::NamedTempFile;
use uuid::Uuid;

use crate::pricing::{Comparison, Cost, Rates};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Preferences {
    pub language: String,
    pub pro: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Metadata {
    pub model: String,
    pub prompt: String,
    pub size: String,
    pub quality: String,
    pub background: String,
    pub format: String,
    pub complexity: Option<String>,
    pub references: Vec<String>,
    pub requested: u64,
    #[serde(default = "standard")]
    pub mode: String,
    pub rates: Rates,
    pub comparison_rates: Comparison,
    pub pricing_source: String,
    pub pricing_checked: String,
    pub pricing_basis: String,
    pub uplift_pct: f64,
}

fn standard() -> String {
    "Standard".into()
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Record {
    #[serde(flatten)]
    pub metadata: Metadata,
    pub record_id: String,
    pub time: String,
    pub files: Vec<String>,
    pub returned_images: Option<usize>,
    pub usage: Value,
    pub cost: Option<Cost>,
    pub error: Value,
    pub output_errors: Vec<String>,
    #[serde(flatten)]
    pub details: Map<String, Value>,
}

impl Record {
    pub fn total(&self) -> Option<f64> {
        self.cost.as_ref().and_then(|cost| cost.total)
    }
    pub fn image_count(&self) -> usize {
        self.returned_images.unwrap_or(self.files.len())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Job {
    pub local_id: String,
    pub time: String,
    pub status: String,
    pub endpoint: String,
    pub metadata: Metadata,
    pub requests: BTreeMap<String, String>,
    pub request_metadata: BTreeMap<String, Metadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_file_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_id: Option<String>,
    pub batch: Value,
    pub imported_files: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(flatten)]
    pub details: Map<String, Value>,
}

#[derive(Clone, Debug)]
pub struct Store {
    pub root: PathBuf,
    pub output: PathBuf,
    history: PathBuf,
    jobs: PathBuf,
    _lock: Arc<File>,
}

impl Store {
    pub fn read_preferences(&self) -> Result<Preferences> {
        match File::open(self.root.join("settings.json")) {
            Ok(file) => Ok(serde_json::from_reader(BufReader::new(file))?),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Preferences::default()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn save_preferences(&self, preferences: &Preferences) -> Result<()> {
        write_json(&self.root, &self.root.join("settings.json"), preferences)
    }

    pub fn beside_exe() -> Result<Self> {
        let exe = std::env::current_exe().context("Could not locate the running executable")?;
        let folder = exe
            .parent()
            .context("Could not find the executable folder")?;
        Self::open(folder)
    }

    pub fn open(folder: &Path) -> Result<Self> {
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(folder.join(".image-cost-studio.lock"))
            .with_context(|| {
                format!(
                    "The app needs a writable folder. Could not open {}",
                    folder.display()
                )
            })?;
        lock.try_lock().with_context(|| {
            format!(
                "Could not lock {}. Close any other copy of the app running from this folder.",
                folder.display()
            )
        })?;
        let output = folder.join("outputs");
        let jobs = folder.join("batches");
        let history = folder.join("history.jsonl");
        for dir in [&output, &jobs] {
            fs::create_dir_all(dir)
                .with_context(|| format!("Could not create {}", dir.display()))?;
            NamedTempFile::new_in(dir)
                .with_context(|| format!("Could not write to {}", dir.display()))?;
        }
        OpenOptions::new()
            .append(true)
            .create(true)
            .open(&history)
            .with_context(|| format!("Could not open {} for writing", history.display()))?;
        Ok(Self {
            root: folder.into(),
            output,
            history,
            jobs,
            _lock: Arc::new(lock),
        })
    }

    pub fn read_history(&self) -> Result<(Vec<Record>, usize)> {
        let mut records = Vec::new();
        let mut skipped = 0;
        let reader = BufReader::new(File::open(&self.history)?);
        for line in reader.split(b'\n') {
            let line = line?;
            let line = line.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&line);
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            match serde_json::from_slice::<Record>(line) {
                Ok(record) if !record.metadata.model.is_empty() => records.push(record),
                _ => skipped += 1,
            }
        }
        records.reverse();
        Ok((records, skipped))
    }

    pub fn append_history(&self, record: &Record) -> Result<()> {
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .open(&self.history)?;
        let mut line = Vec::new();
        if file.metadata()?.len() > 0 {
            file.seek(SeekFrom::End(-1))?;
            let mut last = [0];
            file.read_exact(&mut last)?;
            if last[0] != b'\n' {
                line.push(b'\n');
            }
        }
        serde_json::to_writer(&mut line, record)?;
        line.push(b'\n');
        file.write_all(&line)
            .context("Could not append to history.jsonl")?;
        file.sync_data()?;
        Ok(())
    }

    pub fn read_jobs(&self) -> Result<Vec<Job>> {
        let mut jobs = Vec::new();
        for file in fs::read_dir(&self.jobs)? {
            let path = file?.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Ok(source) = File::open(&path) else {
                continue;
            };
            if let Ok(job) = serde_json::from_reader::<_, Job>(BufReader::new(source))
                && Uuid::parse_str(&job.local_id).is_ok()
            {
                jobs.push(job);
            }
        }
        jobs.sort_by(|a, b| b.time.cmp(&a.time));
        Ok(jobs)
    }

    pub fn save_job(&self, job: &Job) -> Result<()> {
        Uuid::parse_str(&job.local_id).context("Invalid local Batch job ID")?;
        write_json(
            &self.jobs,
            &self.jobs.join(format!("{}.json", job.local_id)),
            job,
        )
    }

    pub fn image_path(&self, filename: &str) -> Option<PathBuf> {
        Some(self.output.join(Path::new(filename).file_name()?))
    }

    pub fn save_image(
        &self,
        id: &str,
        index: usize,
        extension: &str,
        bytes: &[u8],
    ) -> Result<String> {
        Uuid::parse_str(id).context("Invalid image record ID")?;
        ensure!(
            ["png", "jpeg", "webp"].contains(&extension),
            "Unsupported output format"
        );
        let mut temp = NamedTempFile::new_in(&self.output)?;
        temp.write_all(bytes)?;
        temp.as_file().sync_all()?;
        let mut filename = format!("{id}_{index}.{extension}");
        loop {
            let target = self.output.join(&filename);
            match temp.persist_noclobber(&target) {
                Ok(_) => return Ok(filename),
                Err(err) if err.error.kind() == io::ErrorKind::AlreadyExists => {
                    if fs::read(&target).is_ok_and(|existing| existing == bytes) {
                        return Ok(filename);
                    }
                    temp = err.file;
                    filename = format!("{id}_{index}_{}.{extension}", Uuid::new_v4().simple());
                }
                Err(err) => return Err(err.into()),
            }
        }
    }
}

fn write_json(directory: &Path, target: &Path, value: &impl Serialize) -> Result<()> {
    let mut file = NamedTempFile::new_in(directory)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.as_file().sync_all()?;
    file.persist(target)?;
    Ok(())
}

pub fn now() -> String {
    Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_collision_preserves_existing_file() {
        let folder = tempfile::tempdir().unwrap();
        let store = Store::open(folder.path()).unwrap();
        let id = Uuid::new_v4().simple().to_string();
        let first = store.save_image(&id, 1, "png", b"first").unwrap();
        assert_eq!(store.save_image(&id, 1, "png", b"first").unwrap(), first);
        let second = store.save_image(&id, 1, "png", b"second").unwrap();
        assert_ne!(first, second);
        assert_eq!(fs::read(store.output.join(first)).unwrap(), b"first");
        assert_eq!(fs::read(store.output.join(second)).unwrap(), b"second");
        assert!(Store::open(folder.path()).is_err());
    }

    #[test]
    fn appends_after_legacy_history_without_newline() {
        let folder = tempfile::tempdir().unwrap();
        let store = Store::open(folder.path()).unwrap();
        fs::write(
            &store.history,
            r#"{"model":"legacy","prompt":"old","cost":{"total":0.1},"files":[]}"#,
        )
        .unwrap();
        let record = Record {
            metadata: Metadata {
                model: "new".into(),
                ..Metadata::default()
            },
            ..Record::default()
        };
        store.append_history(&record).unwrap();
        let (records, skipped) = store.read_history().unwrap();
        assert_eq!(skipped, 0);
        assert_eq!(records.len(), 2);
        assert_eq!(records[1].total(), Some(0.1));
        assert_eq!(records[1].metadata.mode, "Standard");
    }
}
