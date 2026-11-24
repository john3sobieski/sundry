//! The image workspace: the batch list, previews, results and saving.

use std::cell::{Cell, RefCell};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};

use slint::{ComponentHandle, Model, Rgba8Pixel, SharedPixelBuffer, VecModel};
use sundry_archive::{ArchiveFormat, Entry, unique_names, write_archive};
use sundry_core::{BatchSource, Data, Error, Job, Output, Params, Tool, run_batch};
use sundry_tools_image::codec::INPUT_EXTENSIONS;
use sundry_tools_image::preview;

use crate::{AppWindow, FileRow, Workspace};

/// One processed file, kept in memory until it is saved.
pub struct ResultEntry {
    pub name: String,
    pub data: Vec<u8>,
    detail: String,
}

enum Event {
    Item(usize, Result<ResultEntry, String>),
    Finished,
}

/// A decoded picture on its way to the window (the `Image` itself must be made on the UI thread).
struct Prepared {
    buffer: SharedPixelBuffer<Rgba8Pixel>,
    info: String,
}

#[derive(Default)]
pub struct Session {
    files: RefCell<Vec<PathBuf>>,
    rows: Rc<VecModel<FileRow>>,
    results: RefCell<Vec<Option<Arc<ResultEntry>>>>,
    selected: Cell<Option<usize>>,
    /// Bumped on every selection; a preview that arrives late is dropped.
    seq: Arc<AtomicU64>,
    events: RefCell<Option<mpsc::Receiver<Event>>>,
    running: Cell<bool>,
    done: Cell<usize>,
    failed: Cell<usize>,
}

pub fn human_size(bytes: u64) -> String {
    const KB: f64 = 1000.0;
    let b = bytes as f64;
    if b < KB {
        format!("{bytes} B")
    } else if b < KB * KB {
        format!("{:.0} kB", b / KB)
    } else if b < KB * KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else {
        format!("{:.2} GB", b / (KB * KB * KB))
    }
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
}

pub fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| INPUT_EXTENSIONS.contains(&e.to_lowercase().as_str()))
}

fn prepare(bytes: &[u8], file_len: u64) -> Result<Prepared, String> {
    let p = preview::decode_preview(bytes).map_err(|e| e.to_string())?;
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(p.width, p.height);
    buffer.make_mut_bytes().copy_from_slice(&p.rgba);
    let (w, h) = p.original;
    let info = format!("{w} \u{d7} {h} \u{b7} {} \u{b7} {}", p.format, human_size(file_len));
    Ok(Prepared { buffer, info })
}

/// Hands a finished preview to the window, unless the selection has moved on.
fn deliver(
    weak: slint::Weak<AppWindow>,
    seq: Arc<AtomicU64>,
    mine: u64,
    input: bool,
    result: Result<Prepared, String>,
) {
    let _ = slint::invoke_from_event_loop(move || {
        let Some(ui) = weak.upgrade() else { return };
        if seq.load(Ordering::Relaxed) != mine {
            return;
        }
        let ws = ui.global::<Workspace>();
        match (input, result) {
            (true, Ok(p)) => {
                ws.set_input_image(slint::Image::from_rgba8(p.buffer));
                ws.set_input_info(p.info.into());
                ws.set_input_loaded(true);
                ws.set_input_state("ready".into());
            }
            (true, Err(message)) => {
                ws.set_input_loaded(false);
                ws.set_input_error(message.into());
                ws.set_input_state("error".into());
            }
            (false, Ok(p)) => {
                ws.set_output_image(slint::Image::from_rgba8(p.buffer));
                ws.set_output_info(p.info.into());
                ws.set_output_loaded(true);
                ws.set_output_state("ready".into());
            }
            (false, Err(message)) => {
                ws.set_output_loaded(false);
                ws.set_output_error(message.into());
                ws.set_output_state("error".into());
            }
        }
    });
}

fn entry_of(output: Output) -> Result<ResultEntry, String> {
    let item = output.items.into_iter().next().ok_or_else(|| "empty result".to_owned())?;
    let data = match item.data {
        Data::Bytes(bytes) => bytes,
        other => other.as_bytes().to_vec(),
    };
    let weight = human_size(data.len() as u64);
    let detail = match preview::describe(&data) {
        Some(((w, h), _)) => format!("{w} \u{d7} {h} \u{b7} {weight}"),
        None => weight,
    };
    Ok(ResultEntry { name: item.name.unwrap_or_else(|| "image".to_owned()), data, detail })
}

fn set_note(weak: &slint::Weak<AppWindow>, kind: &str, arg: &str) {
    if let Some(ui) = weak.upgrade() {
        let ws = ui.global::<Workspace>();
        ws.set_note_kind(kind.into());
        ws.set_note_arg(arg.into());
    }
}

fn pack(entries: &[Arc<ResultEntry>], format: ArchiveFormat, path: &Path) -> Result<usize, String> {
    let names = unique_names(entries.iter().map(|e| Some(e.name.as_str())));
    let items: Vec<Entry<'_>> =
        names.iter().zip(entries).map(|(name, e)| Entry { name, data: &e.data }).collect();
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let written = write_archive(format, &items, BufWriter::new(file), |_| true)
        .map_err(|e| e.to_string())
        .and_then(|mut writer| writer.flush().map_err(|e| e.to_string()));
    match written {
        Ok(()) => Ok(items.len()),
        Err(message) => {
            let _ = std::fs::remove_file(path);
            Err(message)
        }
    }
}

impl Session {
    pub fn rows(&self) -> Rc<VecModel<FileRow>> {
        Rc::clone(&self.rows)
    }

    pub fn paths(&self) -> Vec<PathBuf> {
        self.files.borrow().clone()
    }

    pub fn has_files(&self) -> bool {
        !self.files.borrow().is_empty()
    }

    pub fn add(&self, ui: &AppWindow, paths: Vec<PathBuf>, images_only: bool) {
        if self.running.get() {
            return;
        }
        {
            let mut files = self.files.borrow_mut();
            for path in paths {
                let skip = !path.is_file()
                    || (images_only && !is_image_path(&path))
                    || files.contains(&path);
                if skip {
                    continue;
                }
                let size = std::fs::metadata(&path).map(|m| human_size(m.len()));
                let size = size.unwrap_or_default();
                self.rows.push(FileRow {
                    name: display_name(&path).into(),
                    size: size.into(),
                    status: "pending".into(),
                    ..Default::default()
                });
                self.results.borrow_mut().push(None);
                files.push(path);
            }
        }
        if self.selected.get().is_none() && self.rows.row_count() > 0 {
            self.select(ui, 0);
        }
        self.update_counts(ui);
    }

    pub fn remove(&self, ui: &AppWindow, index: usize) {
        if self.running.get() || index >= self.rows.row_count() {
            return;
        }
        if let Some(row) = self.rows.row_data(index) {
            match row.status.as_str() {
                "ok" => self.done.set(self.done.get().saturating_sub(1)),
                "error" => {
                    self.done.set(self.done.get().saturating_sub(1));
                    self.failed.set(self.failed.get().saturating_sub(1));
                }
                _ => {}
            }
        }
        self.files.borrow_mut().remove(index);
        self.rows.remove(index);
        self.results.borrow_mut().remove(index);
        let len = self.rows.row_count();
        match self.selected.get() {
            _ if len == 0 => self.clear_view(ui),
            Some(s) if s == index => self.select(ui, index.min(len - 1)),
            Some(s) if s > index => {
                self.selected.set(Some(s - 1));
                ui.global::<Workspace>().set_selected(i32::try_from(s - 1).unwrap_or(-1));
            }
            _ => {}
        }
        self.update_counts(ui);
    }

    pub fn clear(&self, ui: &AppWindow) {
        if self.running.get() {
            return;
        }
        self.files.borrow_mut().clear();
        self.rows.set_vec(Vec::<FileRow>::new());
        self.results.borrow_mut().clear();
        self.done.set(0);
        self.failed.set(0);
        self.clear_view(ui);
        self.update_counts(ui);
    }

    /// Empties both viewers and forgets the selection.
    fn clear_view(&self, ui: &AppWindow) {
        self.selected.set(None);
        self.seq.fetch_add(1, Ordering::Relaxed);
        let ws = ui.global::<Workspace>();
        ws.set_selected(-1);
        ws.set_input_loaded(false);
        ws.set_input_state("".into());
        ws.set_output_loaded(false);
        ws.set_output_state("".into());
    }

    pub fn select(&self, ui: &AppWindow, index: usize) {
        let Some(path) = self.files.borrow().get(index).cloned() else { return };
        self.selected.set(Some(index));
        let mine = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        let ws = ui.global::<Workspace>();
        ws.set_selected(i32::try_from(index).unwrap_or(-1));
        ws.set_in_zoom(1.0);
        ws.set_out_zoom(1.0);
        ws.set_in_cx(0.5);
        ws.set_in_cy(0.5);
        ws.set_out_cx(0.5);
        ws.set_out_cy(0.5);
        ws.set_input_loaded(false);
        ws.set_input_state("loading".into());
        let (weak, seq) = (ui.as_weak(), Arc::clone(&self.seq));
        std::thread::spawn(move || {
            let result = std::fs::read(&path)
                .map_err(|e| format!("{}: {e}", path.display()))
                .and_then(|bytes| prepare(&bytes, bytes.len() as u64));
            deliver(weak, seq, mine, true, result);
        });
        self.show_output(ui, index, mine);
        self.update_counts(ui);
    }

    /// Shows the result of file `index` (or why there is none) in the output viewer.
    fn show_output(&self, ui: &AppWindow, index: usize, mine: u64) {
        let ws = ui.global::<Workspace>();
        let result = self.results.borrow().get(index).cloned().flatten();
        if let Some(entry) = result {
            ws.set_output_loaded(false);
            ws.set_output_state("loading".into());
            let (weak, seq) = (ui.as_weak(), Arc::clone(&self.seq));
            std::thread::spawn(move || {
                deliver(weak, seq, mine, false, prepare(&entry.data, entry.data.len() as u64));
            });
            return;
        }
        ws.set_output_loaded(false);
        match self.rows.row_data(index) {
            Some(row) if row.status == "error" => {
                ws.set_output_error(row.detail);
                ws.set_output_state("error".into());
            }
            _ => ws.set_output_state("waiting".into()),
        }
    }

    fn update_counts(&self, ui: &AppWindow) {
        let ws = ui.global::<Workspace>();
        let results = self.results.borrow();
        let count = |n: usize| i32::try_from(n).unwrap_or(i32::MAX);
        ws.set_done_count(count(self.done.get()));
        ws.set_failed_count(count(self.failed.get()));
        ws.set_result_count(count(results.iter().flatten().count()));
        let has = self.selected.get().is_some_and(|i| results.get(i).is_some_and(Option::is_some));
        ws.set_selected_has_result(has);
    }

    /// Runs `tool` on every file in parallel; results arrive through `poll`.
    pub fn start(&self, ui: &AppWindow, tool: Arc<dyn Tool>, params: Params, job: Job) {
        let paths = self.paths();
        self.results.borrow_mut().iter_mut().for_each(|r| *r = None);
        for i in 0..self.rows.row_count() {
            if let Some(mut row) = self.rows.row_data(i) {
                row.status = "pending".into();
                row.detail = "".into();
                self.rows.set_row_data(i, row);
            }
        }
        self.done.set(0);
        self.failed.set(0);
        self.running.set(true);
        ui.global::<Workspace>().set_note_kind("".into());
        if let Some(i) = self.selected.get() {
            self.show_output(ui, i, self.seq.load(Ordering::Relaxed));
        }
        self.update_counts(ui);

        let (tx, rx) = mpsc::channel();
        *self.events.borrow_mut() = Some(rx);
        let sources: Vec<BatchSource> = paths
            .into_iter()
            .map(|path| {
                let name = display_name(&path);
                BatchSource::new(name, move || std::fs::read(&path).map_err(Error::from))
            })
            .collect();
        let workers = std::thread::available_parallelism().map_or(2, usize::from).clamp(1, 6);
        std::thread::spawn(move || {
            run_batch(&*tool, &params, &sources, &job, workers, &|index, result| {
                let entry = result.map_err(|e| e.to_string()).and_then(entry_of);
                let _ = tx.send(Event::Item(index, entry));
            });
            let _ = tx.send(Event::Finished);
        });
    }

    /// Applies the results that arrived since the last call; called by a timer.
    pub fn poll(&self, ui: &AppWindow) {
        let mut changed = false;
        loop {
            let event = match self.events.borrow().as_ref() {
                Some(rx) => rx.try_recv().ok(),
                None => None,
            };
            let Some(event) = event else { break };
            changed = true;
            match event {
                Event::Item(index, outcome) => self.apply_item(ui, index, outcome),
                Event::Finished => {
                    *self.events.borrow_mut() = None;
                    self.running.set(false);
                    ui.set_running(false);
                }
            }
        }
        if changed {
            self.update_counts(ui);
        }
    }

    fn apply_item(&self, ui: &AppWindow, index: usize, outcome: Result<ResultEntry, String>) {
        let Some(mut row) = self.rows.row_data(index) else { return };
        self.done.set(self.done.get() + 1);
        match outcome {
            Ok(entry) => {
                row.status = "ok".into();
                row.detail = entry.detail.clone().into();
                if let Some(slot) = self.results.borrow_mut().get_mut(index) {
                    *slot = Some(Arc::new(entry));
                }
            }
            Err(message) => {
                self.failed.set(self.failed.get() + 1);
                row.status = "error".into();
                row.detail = message.into();
            }
        }
        self.rows.set_row_data(index, row);
        if self.selected.get() == Some(index) {
            self.show_output(ui, index, self.seq.load(Ordering::Relaxed));
        }
    }

    pub fn save_selected(&self, weak: slint::Weak<AppWindow>) {
        let current = self.selected.get();
        let entry = current.and_then(|i| self.results.borrow().get(i).cloned().flatten());
        let Some(entry) = entry else { return };
        let _ = slint::spawn_local(async move {
            let mut dialog = rfd::AsyncFileDialog::new().set_file_name(&entry.name);
            if let Some(ext) = Path::new(&entry.name).extension().and_then(|e| e.to_str()) {
                dialog = dialog.add_filter(ext.to_uppercase(), &[ext]);
            }
            let Some(handle) = dialog.save_file().await else { return };
            match std::fs::write(handle.path(), &entry.data) {
                Ok(()) => set_note(&weak, "saved", &display_name(handle.path())),
                Err(err) => set_note(&weak, "error", &err.to_string()),
            }
        });
    }

    pub fn save_all(&self, weak: slint::Weak<AppWindow>, format_index: i32) {
        let entries: Vec<Arc<ResultEntry>> =
            self.results.borrow().iter().flatten().cloned().collect();
        if entries.is_empty() {
            set_note(&weak, "no-files", "");
            return;
        }
        let format = if format_index == 1 { ArchiveFormat::SevenZ } else { ArchiveFormat::Zip };
        let _ = slint::spawn_local(async move {
            let ext = format.extension();
            let dialog = rfd::AsyncFileDialog::new()
                .set_file_name(format!("sundry-images.{ext}"))
                .add_filter(ext.to_uppercase(), &[ext]);
            let Some(handle) = dialog.save_file().await else { return };
            let path = handle.path().to_path_buf();
            set_note(&weak, "", "");
            if let Some(ui) = weak.upgrade() {
                ui.global::<Workspace>().set_archiving(true);
            }
            std::thread::spawn(move || {
                let outcome = pack(&entries, format, &path);
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = weak.upgrade() else { return };
                    ui.global::<Workspace>().set_archiving(false);
                    match outcome {
                        Ok(count) => set_note(&weak, "saved-all", &count.to_string()),
                        Err(message) => set_note(&weak, "error", &message),
                    }
                });
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_human_readable() {
        assert_eq!(human_size(999), "999 B");
        assert_eq!(human_size(1500), "2 kB");
        assert_eq!(human_size(3_100_000), "3.1 MB");
        assert_eq!(human_size(2_500_000_000), "2.50 GB");
    }

    #[test]
    fn only_image_files_are_picked_up() {
        assert!(is_image_path(Path::new("a/b/Photo.JPG")));
        assert!(is_image_path(Path::new("x.avif")));
        assert!(!is_image_path(Path::new("notes.txt")));
        assert!(!is_image_path(Path::new("noextension")));
    }
}
