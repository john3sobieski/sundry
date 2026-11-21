//! Sundry desktop app: a Slint shell over the tool registry.

#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use sundry_core::{
    Category, Error, ParamKind, ParamSpec, Params, Registry, Settings, Value, logging, settings,
};

slint::include_modules!();

/// Current values of the open tool's parameter form; `None` marks an invalid field.
#[derive(Default)]
struct Form {
    tool_id: String,
    specs: Vec<ParamSpec>,
    values: Vec<Option<Value>>,
}

impl Form {
    fn is_valid(&self) -> bool {
        self.values.iter().all(Option::is_some)
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

/// Models shown by the window; refreshed whenever the category, query or tool changes.
struct Shell {
    registry: Registry,
    categories: Rc<VecModel<CategoryItem>>,
    tools: Rc<VecModel<ToolItem>>,
    fields: Rc<VecModel<FormField>>,
    form: RefCell<Form>,
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
                }
            })
            .collect();
        self.tools.set_vec(tools);
    }

    /// Builds the parameter form of a tool from its specs.
    fn open_tool(&self, id: &str) {
        let entry = match self.registry.get(id) {
            Ok(entry) => entry,
            Err(err) => {
                tracing::warn!("cannot open tool: {err}");
                return;
            }
        };
        let specs = entry.params().to_vec();
        let values = specs.iter().map(|s| Some(s.default_value())).collect();
        self.fields.set_vec(specs.iter().map(field_of).collect::<Vec<_>>());
        *self.form.borrow_mut() = Form { tool_id: id.to_owned(), specs, values };
    }

    /// Validates one edited field and shows the problem (if any) under it.
    fn edit(&self, index: usize, edit: Edit) {
        let mut form = self.form.borrow_mut();
        let result = match form.specs.get(index) {
            Some(spec) => evaluate(spec, &edit),
            None => return,
        };
        let error = result.as_ref().err().cloned().unwrap_or_default();
        form.values[index] = result.ok();

        if let Some(mut row) = self.fields.row_data(index) {
            match edit {
                Edit::Text(text) => row.text = text.into(),
                Edit::Bool(checked) => row.checked = checked,
                Edit::Choice(choice) => row.index = count(choice),
            }
            row.error = error.into();
            self.fields.set_row_data(index, row);
        }
        tracing::debug!(tool = form.tool_id.as_str(), params = ?form.params(), "form updated");
    }

    fn form_valid(&self) -> bool {
        self.form.borrow().is_valid()
    }
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

fn range_hint(lo: Option<String>, hi: Option<String>) -> String {
    match (lo, hi) {
        (Some(lo), Some(hi)) => format!("{lo} to {hi}"),
        (Some(lo), None) => format!("from {lo}"),
        (None, Some(hi)) => format!("up to {hi}"),
        (None, None) => String::new(),
    }
}

fn field_of(spec: &ParamSpec) -> FormField {
    let mut field = FormField {
        id: spec.id.as_str().into(),
        label: label_of(&spec.id).into(),
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
            field.hint = range_hint(min.map(|v| v.to_string()), max.map(|v| v.to_string())).into();
        }
        ParamKind::Float { default, min, max } => {
            field.kind = "float".into();
            field.text = default.to_string().into();
            field.hint = range_hint(min.map(|v| v.to_string()), max.map(|v| v.to_string())).into();
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

/// Turns raw input into a value and checks it against the spec.
fn evaluate(spec: &ParamSpec, edit: &Edit) -> Result<Value, String> {
    let value = match (&spec.kind, edit) {
        (ParamKind::Int { .. }, Edit::Text(text)) => {
            Value::Int(text.trim().parse().map_err(|_| "Enter a whole number".to_owned())?)
        }
        (ParamKind::Float { .. }, Edit::Text(text)) => {
            // Accept a decimal comma as well as a point.
            let text = text.trim().replace(',', ".");
            Value::Float(text.parse().map_err(|_| "Enter a number".to_owned())?)
        }
        (ParamKind::Text { .. }, Edit::Text(text)) => Value::Text(text.clone()),
        (ParamKind::Bool { .. }, Edit::Bool(checked)) => Value::Bool(*checked),
        (ParamKind::Choice { options, .. }, Edit::Choice(index)) => {
            Value::Text(options.get(*index).cloned().unwrap_or_default())
        }
        _ => return Err("Unexpected input".to_owned()),
    };
    spec.check(&value).map_err(|err| match err {
        Error::InvalidParam { reason, .. } => reason,
        other => other.to_string(),
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

/// Connects the form callbacks of the window to the shell.
fn bind_form(ui: &AppWindow, shell: &Rc<Shell>) {
    let weak = ui.as_weak();
    ui.on_tool_selected({
        let (shell, weak) = (Rc::clone(shell), weak.clone());
        move |tool| {
            shell.open_tool(tool.id.as_str());
            if let Some(ui) = weak.upgrade() {
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
    });
    shell.refresh("", "");

    let ui = AppWindow::new()?;
    ui.set_categories(ModelRc::from(shell.categories.clone()));
    ui.set_tools(ModelRc::from(shell.tools.clone()));
    ui.set_form(ModelRc::from(shell.fields.clone()));
    ui.set_tool_total(count(shell.registry.len()));

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

    ui.on_theme_selected(move |index| {
        let mut config = load_settings();
        config.theme = theme_of(index);
        let saved = settings::default_path().and_then(|path| config.save(&path));
        if let Err(err) = saved {
            tracing::warn!("cannot save settings: {err}");
        }
    });

    ui.run()?;
    Ok(())
}
