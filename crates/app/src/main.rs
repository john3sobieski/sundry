//! Sundry desktop app: a Slint shell over the tool registry.

#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

use std::cell::RefCell;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use sundry_core::{
    Category, Error, Input, InputKind, Item, Job, Language, Media, ParamKind, ParamSpec, Params,
    Registry, Settings, Tool, Value, logging, params, settings,
};

mod session;

use session::Session;

slint::include_modules!();

/// Current values of the open tool's parameter form; `None` marks an invalid field.
#[derive(Default)]
struct Form {
    tool_id: String,
    specs: Vec<ParamSpec>,
    values: Vec<Option<Value>>,
    /// Which fields are shown; a hidden field may hold a bad value without blocking Run.
    shown: Vec<bool>,
}

impl Form {
    fn is_valid(&self) -> bool {
        self.values.iter().zip(&self.shown).all(|(value, shown)| value.is_some() || !shown)
    }

    fn value_of(&self, id: &str) -> Option<Value> {
        let index = self.specs.iter().position(|s| s.id == id)?;
        self.values[index].clone()
    }

    fn visibility(&self) -> Vec<bool> {
        self.specs.iter().map(|spec| spec.is_shown(|id| self.value_of(id))).collect()
    }

    fn params(&self) -> Params {
        let mut params = Params::new();
        for (spec, value) in self.specs.iter().zip(&self.values) {
            if let Some(value) = value {
                params.set(spec.id.clone(), value.clone());
            }
        }
        params
    }
}

/// One user action on a form field.
enum Edit {
    Text(String),
    Bool(bool),
    Choice(usize),
}

/// How a background run ended.
enum Outcome {
    Done(String),
    Failed(String),
    Binary,
    Cancelled,
}

/// A validation problem: a code that the UI turns into text, and its argument.
struct FieldError {
    code: &'static str,
    arg: String,
}

impl FieldError {
    fn new(code: &'static str, arg: impl Into<String>) -> Self {
        Self { code, arg: arg.into() }
    }
}

/// Models shown by the window; refreshed whenever the category, query or tool changes.
struct Shell {
    registry: Registry,
    categories: Rc<VecModel<CategoryItem>>,
    tools: Rc<VecModel<ToolItem>>,
    fields: Rc<VecModel<FormField>>,
    form: RefCell<Form>,
    session: Rc<Session>,
    job: RefCell<Option<Job>>,
}

impl Shell {
    fn refresh(&self, category: &str, query: &str) {
        let matches = self.registry.search(query);

        let mut categories = vec![CategoryItem { key: "".into(), count: count(matches.len()) }];
        for c in Category::ALL {
            let n = matches.iter().filter(|e| e.info().category == c).count();
            categories.push(CategoryItem { key: c.as_str().into(), count: count(n) });
        }
        self.categories.set_vec(categories);

        let tools: Vec<ToolItem> = matches
            .iter()
            .filter(|e| category.is_empty() || e.info().category.as_str() == category)
            .map(|e| {
                let info = e.info();
                ToolItem {
                    id: info.id.as_str().into(),
                    name: info.name.as_str().into(),
                    description: info.description.as_str().into(),
                    input: info.input.as_str().into(),
                    media: info.media.as_str().into(),
                }
            })
            .collect();
        self.tools.set_vec(tools);
    }

    /// Builds the parameter form of a tool from its specs.
    fn open_tool(&self, ui: &AppWindow, id: &str) {
        let entry = match self.registry.get(id) {
            Ok(entry) => entry,
            Err(err) => {
                tracing::warn!("cannot open tool: {err}");
                return;
            }
        };
        if !entry.info().input.accepts_files() {
            self.session.clear(ui);
        }
        let specs = entry.params().to_vec();
        let values = specs.iter().map(|s| Some(s.default_value())).collect();
        self.fields.set_vec(specs.iter().map(field_of).collect::<Vec<_>>());
        *self.form.borrow_mut() = Form { tool_id: id.to_owned(), specs, values, shown: Vec::new() };
        self.refresh_visibility();
    }

    /// Shows or hides fields that depend on other fields (see `ParamSpec::show_when`).
    fn refresh_visibility(&self) {
        let shown = {
            let mut form = self.form.borrow_mut();
            form.shown = form.visibility();
            form.shown.clone()
        };
        for (index, visible) in shown.into_iter().enumerate() {
            if let Some(mut row) = self.fields.row_data(index)
                && row.visible != visible
            {
                row.visible = visible;
                self.fields.set_row_data(index, row);
            }
        }
    }

    /// Validates one edited field and shows the problem (if any) under it.
    fn edit(&self, index: usize, edit: Edit) {
        let mut form = self.form.borrow_mut();
        let result = match form.specs.get(index) {
            Some(spec) => evaluate(spec, &edit),
            None => return,
        };
        let (code, arg) = match &result {
            Ok(_) => ("", String::new()),
            Err(err) => (err.code, err.arg.clone()),
        };
        form.values[index] = result.ok();

        if let Some(mut row) = self.fields.row_data(index) {
            match edit {
                Edit::Text(text) => row.text = text.into(),
                Edit::Bool(checked) => row.checked = checked,
                Edit::Choice(choice) => row.index = count(choice),
            }
            row.error_code = code.into();
            row.error_arg = arg.into();
            self.fields.set_row_data(index, row);
        }
        tracing::debug!(tool = form.tool_id.as_str(), params = ?form.params(), "form updated");
        drop(form);
        self.refresh_visibility();
    }

    fn form_valid(&self) -> bool {
        self.form.borrow().is_valid()
    }

    /// What input the open tool reads.
    fn input_kind(&self) -> InputKind {
        let form = self.form.borrow();
        self.registry.get(&form.tool_id).map(|e| e.info().input).unwrap_or_default()
    }

    /// The kind of files the open tool works on.
    fn media(&self) -> Media {
        let form = self.form.borrow();
        self.registry.get(&form.tool_id).map(|e| e.info().media).unwrap_or_default()
    }

    fn add_files(&self, ui: &AppWindow, paths: Vec<PathBuf>) {
        if !self.input_kind().accepts_files() {
            return;
        }
        self.session.add(ui, paths, self.media() == Media::Image);
    }

    /// Validates the form and starts the tool on a background thread.
    fn start_run(&self, ui: &AppWindow) {
        let prepared = {
            let form = self.form.borrow();
            self.registry.get(&form.tool_id).map_err(|e| e.to_string()).and_then(|entry| {
                let resolved =
                    params::resolve(&form.specs, &form.params()).map_err(|e| e.to_string())?;
                Ok((entry.tool(), resolved, entry.info().per_file))
            })
        };
        let (tool, resolved, per_file) = match prepared {
            Ok(prepared) => prepared,
            Err(message) => {
                ui.set_result_kind("error".into());
                ui.set_result_text(message.into());
                return;
            }
        };

        let job = Job::new();
        *self.job.borrow_mut() = Some(job.clone());
        ui.set_progress(0.0);
        if per_file {
            if !self.session.has_files() {
                ui.global::<Workspace>().set_note_kind("no-files".into());
                return;
            }
            ui.set_running(true);
            self.session.start(ui, tool, resolved, job);
            return;
        }
        ui.set_result_text("".into());
        ui.set_result_kind("".into());
        ui.set_running(true);

        // Input the tool does not read is left out, even if it is still in the window.
        let kind = self.input_kind();
        let text =
            if kind.accepts_text() { ui.get_input_text().to_string() } else { String::new() };
        let files = if kind.accepts_files() { self.session.paths() } else { Vec::new() };
        let weak = ui.as_weak();
        std::thread::spawn(move || {
            let outcome = run_blocking(&*tool, &resolved, &text, &files, &job);
            // The window may be gone by now; then there is nothing to update.
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = weak.upgrade() else { return };
                ui.set_running(false);
                let (kind, text) = match outcome {
                    Outcome::Done(text) if text.is_empty() => ("empty", String::new()),
                    Outcome::Done(text) => ("text", text),
                    Outcome::Failed(message) => ("error", message),
                    Outcome::Binary => ("binary", String::new()),
                    Outcome::Cancelled => ("cancelled", String::new()),
                };
                ui.set_result_text(text.into());
                ui.set_result_kind(kind.into());
            });
        });
    }

    fn cancel_run(&self) {
        if let Some(job) = self.job.borrow().as_ref() {
            job.cancel();
        }
    }
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
}

/// Reads the inputs and runs the tool; meant for a worker thread.
fn run_blocking(
    tool: &dyn Tool,
    params: &Params,
    text: &str,
    files: &[PathBuf],
    job: &Job,
) -> Outcome {
    let mut items = Vec::new();
    if !text.is_empty() {
        items.push(Item::text(text));
    }
    for path in files {
        if job.is_cancelled() {
            return Outcome::Cancelled;
        }
        match std::fs::read(path) {
            Ok(bytes) => items.push(Item::bytes(bytes).named(display_name(path))),
            Err(err) => return Outcome::Failed(format!("{}: {err}", path.display())),
        }
    }

    let output = match tool.run(&Input { items }, params, job) {
        Ok(output) => output,
        Err(Error::Cancelled) => return Outcome::Cancelled,
        Err(err) => return Outcome::Failed(err.to_string()),
    };
    job.set_progress(1.0);

    let mut texts = Vec::with_capacity(output.items.len());
    for item in &output.items {
        match item.data.as_text() {
            Ok(text) => texts.push(text),
            Err(_) => return Outcome::Binary,
        }
    }
    Outcome::Done(texts.join("\n"))
}

/// Local file paths carried by a drag-and-drop payload.
fn paths_of(data: &slint::DataTransfer) -> Vec<PathBuf> {
    data.file_paths().map(|it| it.map(Path::to_path_buf).collect()).unwrap_or_default()
}

fn count(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

/// `chunk_size` -> `Chunk size`. Real labels come with localization.
fn label_of(id: &str) -> String {
    let spaced = id.replace('_', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn bound_text<T: Display>(bound: &Option<T>) -> SharedString {
    bound.as_ref().map(|v| v.to_string()).unwrap_or_default().into()
}

fn field_of(spec: &ParamSpec) -> FormField {
    let mut field = FormField {
        id: spec.id.as_str().into(),
        label: label_of(&spec.id).into(),
        visible: true,
        ..Default::default()
    };
    match &spec.kind {
        ParamKind::Bool { default } => {
            field.kind = "bool".into();
            field.checked = *default;
        }
        ParamKind::Int { default, min, max } => {
            field.kind = "int".into();
            field.text = default.to_string().into();
            field.min = bound_text(min);
            field.max = bound_text(max);
        }
        ParamKind::Float { default, min, max } => {
            field.kind = "float".into();
            field.text = default.to_string().into();
            field.min = bound_text(min);
            field.max = bound_text(max);
        }
        ParamKind::Text { default } => {
            field.kind = "text".into();
            field.text = default.as_str().into();
        }
        ParamKind::Choice { options, default } => {
            field.kind = "choice".into();
            field.index = count(options.iter().position(|o| o == default).unwrap_or(0));
            let labels: Vec<SharedString> = options.iter().map(|o| o.as_str().into()).collect();
            field.options = ModelRc::from(Rc::new(VecModel::from(labels)));
        }
    }
    field
}

/// Reports the first violated bound, as a code for the UI.
fn check_bounds<T: PartialOrd + Display>(
    value: &T,
    min: &Option<T>,
    max: &Option<T>,
) -> Result<(), FieldError> {
    if let Some(lo) = min
        && value < lo
    {
        return Err(FieldError::new("below-min", lo.to_string()));
    }
    if let Some(hi) = max
        && value > hi
    {
        return Err(FieldError::new("above-max", hi.to_string()));
    }
    Ok(())
}

/// Turns raw input into a value and checks it against the spec.
fn evaluate(spec: &ParamSpec, edit: &Edit) -> Result<Value, FieldError> {
    let value = match (&spec.kind, edit) {
        (ParamKind::Int { min, max, .. }, Edit::Text(text)) => {
            let n: i64 = text.trim().parse().map_err(|_| FieldError::new("whole-number", ""))?;
            check_bounds(&n, min, max)?;
            Value::Int(n)
        }
        (ParamKind::Float { min, max, .. }, Edit::Text(text)) => {
            // Accept a decimal comma as well as a point.
            let n: f64 =
                text.trim().replace(',', ".").parse().map_err(|_| FieldError::new("number", ""))?;
            check_bounds(&n, min, max)?;
            Value::Float(n)
        }
        (ParamKind::Text { .. }, Edit::Text(text)) => Value::Text(text.clone()),
        (ParamKind::Bool { .. }, Edit::Bool(checked)) => Value::Bool(*checked),
        (ParamKind::Choice { options, .. }, Edit::Choice(index)) => {
            Value::Text(options.get(*index).cloned().unwrap_or_default())
        }
        _ => return Err(FieldError::new("unexpected", "")),
    };
    // Anything the checks above did not catch: the core's own (English) reason.
    spec.check(&value).map_err(|err| match err {
        Error::InvalidParam { reason, .. } => FieldError::new("invalid", reason),
        other => FieldError::new("invalid", other.to_string()),
    })
}

fn index_of(i: i32) -> Option<usize> {
    usize::try_from(i).ok()
}

fn mode_of(theme: sundry_core::Theme) -> (ThemeMode, i32) {
    match theme {
        sundry_core::Theme::Auto => (ThemeMode::System, 0),
        sundry_core::Theme::Light => (ThemeMode::Light, 1),
        sundry_core::Theme::Dark => (ThemeMode::Dark, 2),
    }
}

fn theme_of(index: i32) -> sundry_core::Theme {
    match index {
        1 => sundry_core::Theme::Light,
        2 => sundry_core::Theme::Dark,
        _ => sundry_core::Theme::Auto,
    }
}

fn language_index(language: Language) -> i32 {
    match language {
        Language::En => 0,
        Language::Ru => 1,
        Language::Auto => {
            let russian =
                sys_locale::get_locale().is_some_and(|l| l.to_lowercase().starts_with("ru"));
            i32::from(russian)
        }
    }
}

fn language_of(index: i32) -> Language {
    if index == 1 { Language::Ru } else { Language::En }
}

fn apply_language(language: Language) {
    let code = if language_index(language) == 1 { "ru" } else { "en" };
    if let Err(err) = slint::select_bundled_translation(code) {
        tracing::warn!("cannot select language {code}: {err:?}");
    }
}

/// Re-reads the settings file, applies `change` and writes it back.
fn update_settings(change: impl FnOnce(&mut Settings)) {
    let mut config = load_settings();
    change(&mut config);
    let saved = settings::default_path().and_then(|path| config.save(&path));
    if let Err(err) = saved {
        tracing::warn!("cannot save settings: {err}");
    }
}

/// A broken settings file must not stop the app: warn and use defaults.
fn load_settings() -> Settings {
    let Ok(path) = settings::default_path() else {
        return Settings::default();
    };
    Settings::load(&path).unwrap_or_else(|err| {
        eprintln!("warning: ignoring {}: {err}", path.display());
        Settings::default()
    })
}

/// Opens the file picker; image tools only offer image files.
fn request_files(shell: &Rc<Shell>, weak: slint::Weak<AppWindow>) {
    let shell = Rc::clone(shell);
    // The dialog runs on the event loop without blocking it; the handle is
    // dropped on purpose, which detaches the task.
    let _ = slint::spawn_local(async move {
        let mut dialog = rfd::AsyncFileDialog::new();
        if shell.media() == Media::Image {
            dialog = dialog.add_filter("Images", sundry_tools_image::codec::INPUT_EXTENSIONS);
        }
        if let Some(files) = dialog.pick_files().await
            && let Some(ui) = weak.upgrade()
        {
            shell.add_files(&ui, files.iter().map(|f| f.path().to_path_buf()).collect());
        }
    });
}

/// Connects dropped files, run and cancel to the shell.
fn bind_run(ui: &AppWindow, shell: &Rc<Shell>) {
    let api = ui.global::<DropApi>();
    api.on_can_drop({
        let shell = Rc::clone(shell);
        move |data| shell.input_kind().accepts_files() && !paths_of(&data).is_empty()
    });
    api.on_dropped({
        let (shell, weak) = (Rc::clone(shell), ui.as_weak());
        move |data| {
            if let Some(ui) = weak.upgrade() {
                shell.add_files(&ui, paths_of(&data));
            }
        }
    });

    ui.on_file_removed({
        let (shell, weak) = (Rc::clone(shell), ui.as_weak());
        move |i| {
            if let (Some(i), Some(ui)) = (index_of(i), weak.upgrade()) {
                shell.session.remove(&ui, i);
            }
        }
    });
    ui.on_files_requested({
        let (shell, weak) = (Rc::clone(shell), ui.as_weak());
        move || request_files(&shell, weak.clone())
    });
    ui.on_files_cleared({
        let (shell, weak) = (Rc::clone(shell), ui.as_weak());
        move || {
            if let Some(ui) = weak.upgrade() {
                shell.session.clear(&ui);
            }
        }
    });

    let workspace = ui.global::<Workspace>();
    workspace.on_file_selected({
        let (shell, weak) = (Rc::clone(shell), ui.as_weak());
        move |i| {
            if let (Some(i), Some(ui)) = (index_of(i), weak.upgrade()) {
                shell.session.select(&ui, i);
            }
        }
    });
    workspace.on_add_requested({
        let (shell, weak) = (Rc::clone(shell), ui.as_weak());
        move || request_files(&shell, weak.clone())
    });
    workspace.on_clear_requested({
        let (shell, weak) = (Rc::clone(shell), ui.as_weak());
        move || {
            if let Some(ui) = weak.upgrade() {
                shell.session.clear(&ui);
            }
        }
    });
    workspace.on_remove_requested({
        let (shell, weak) = (Rc::clone(shell), ui.as_weak());
        move |i| {
            if let (Some(i), Some(ui)) = (index_of(i), weak.upgrade()) {
                shell.session.remove(&ui, i);
            }
        }
    });
    workspace.on_save_one({
        let (shell, weak) = (Rc::clone(shell), ui.as_weak());
        move || shell.session.save_selected(weak.clone())
    });
    workspace.on_save_all({
        let (shell, weak) = (Rc::clone(shell), ui.as_weak());
        move |format| shell.session.save_all(weak.clone(), format)
    });

    ui.on_run_requested({
        let (shell, weak) = (Rc::clone(shell), ui.as_weak());
        move || {
            if let Some(ui) = weak.upgrade() {
                shell.start_run(&ui);
            }
        }
    });
    ui.on_cancel_requested({
        let shell = Rc::clone(shell);
        move || shell.cancel_run()
    });
}

/// Connects the form callbacks of the window to the shell.
fn bind_form(ui: &AppWindow, shell: &Rc<Shell>) {
    let weak = ui.as_weak();
    ui.on_tool_selected({
        let (shell, weak) = (Rc::clone(shell), weak.clone());
        move |tool| {
            if let Some(ui) = weak.upgrade() {
                shell.open_tool(&ui, tool.id.as_str());
                ui.set_form_valid(shell.form_valid());
            }
        }
    });

    ui.on_field_edited({
        let (shell, weak) = (Rc::clone(shell), weak.clone());
        move |i, text| {
            if let Some(i) = index_of(i) {
                shell.edit(i, Edit::Text(text.to_string()));
            }
            if let Some(ui) = weak.upgrade() {
                ui.set_form_valid(shell.form_valid());
            }
        }
    });

    ui.on_field_toggled({
        let (shell, weak) = (Rc::clone(shell), weak.clone());
        move |i, checked| {
            if let Some(i) = index_of(i) {
                shell.edit(i, Edit::Bool(checked));
            }
            if let Some(ui) = weak.upgrade() {
                ui.set_form_valid(shell.form_valid());
            }
        }
    });

    ui.on_field_selected({
        let (shell, weak) = (Rc::clone(shell), weak);
        move |i, choice| {
            if let (Some(i), Some(choice)) = (index_of(i), index_of(choice)) {
                shell.edit(i, Edit::Choice(choice));
            }
            if let Some(ui) = weak.upgrade() {
                ui.set_form_valid(shell.form_valid());
            }
        }
    });
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = load_settings();
    if let Err(err) = logging::init(config.log_level) {
        eprintln!("warning: {err}");
    }

    let shell = Rc::new(Shell {
        registry: sundry_cli::builtin::registry()?,
        categories: Rc::new(VecModel::default()),
        tools: Rc::new(VecModel::default()),
        fields: Rc::new(VecModel::default()),
        form: RefCell::new(Form::default()),
        session: Rc::new(Session::default()),
        job: RefCell::new(None),
    });
    shell.refresh("", "");

    let ui = AppWindow::new()?;
    ui.set_categories(ModelRc::from(shell.categories.clone()));
    ui.set_tools(ModelRc::from(shell.tools.clone()));
    ui.set_form(ModelRc::from(shell.fields.clone()));
    ui.set_input_files(ModelRc::from(shell.session.rows()));
    ui.global::<Workspace>().set_files(ModelRc::from(shell.session.rows()));
    ui.set_tool_total(count(shell.registry.len()));

    apply_language(config.language);
    ui.set_language_index(language_index(config.language));
    let (mode, index) = mode_of(config.theme);
    ui.global::<Theme>().set_mode(mode);
    ui.set_theme_index(index);

    let weak = ui.as_weak();
    let on_change = {
        let shell = Rc::clone(&shell);
        move |_: SharedString| {
            if let Some(ui) = weak.upgrade() {
                shell.refresh(&ui.get_selected_category(), &ui.get_query());
            }
        }
    };
    ui.on_category_selected(on_change.clone());
    ui.on_search_edited(on_change);

    bind_form(&ui, &shell);
    bind_run(&ui, &shell);

    // Mirrors the running job's progress into the status bar.
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(50), {
        let (shell, weak) = (Rc::clone(&shell), ui.as_weak());
        move || {
            let Some(ui) = weak.upgrade() else { return };
            shell.session.poll(&ui);
            if ui.get_running()
                && let Some(job) = shell.job.borrow().as_ref()
            {
                ui.set_progress(job.progress());
            }
        }
    });

    ui.on_theme_selected(|index| update_settings(|c| c.theme = theme_of(index)));
    ui.on_language_selected(|index| {
        let language = language_of(index);
        apply_language(language);
        update_settings(|c| c.language = language);
    });

    ui.run()?;
    Ok(())
}
