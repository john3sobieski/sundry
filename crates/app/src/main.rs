//! Sundry desktop app: a Slint shell over the tool registry.

#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, VecModel};
use sundry_core::{Category, Registry, Settings, logging, settings};

slint::include_modules!();

/// Models shown by the window; refreshed whenever the category or query changes.
struct Shell {
    registry: Registry,
    categories: Rc<VecModel<CategoryItem>>,
    tools: Rc<VecModel<ToolItem>>,
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
}

fn count(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = load_settings();
    if let Err(err) = logging::init(config.log_level) {
        eprintln!("warning: {err}");
    }

    let shell = Rc::new(Shell {
        registry: sundry_cli::builtin::registry()?,
        categories: Rc::new(VecModel::default()),
        tools: Rc::new(VecModel::default()),
    });
    shell.refresh("", "");

    let ui = AppWindow::new()?;
    ui.set_categories(ModelRc::from(shell.categories.clone()));
    ui.set_tools(ModelRc::from(shell.tools.clone()));
    ui.set_tool_total(count(shell.registry.len()));

    let (mode, index) = mode_of(config.theme);
    ui.global::<Theme>().set_mode(mode);
    ui.set_theme_index(index);

    let weak = ui.as_weak();
    let on_change = {
        let shell = Rc::clone(&shell);
        move |_: slint::SharedString| {
            if let Some(ui) = weak.upgrade() {
                shell.refresh(&ui.get_selected_category(), &ui.get_query());
            }
        }
    };
    ui.on_category_selected(on_change.clone());
    ui.on_search_edited(on_change);

    ui.on_tool_selected(|tool| tracing::debug!(tool = tool.id.as_str(), "tool selected"));

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
