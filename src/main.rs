use std::{
    cell::RefCell,
    env, fs,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    process::Command,
    rc::Rc,
    time::{SystemTime, UNIX_EPOCH},
};

use slint::{ModelRc, VecModel};

slint::include_modules!();

const PROFILES_SUBDIR: &str = "profiles";
const LINK_NAME: &str = "current-profile.kdl";
const MAIN_CONFIG: &str = "config.kdl";
const APP_ID: &str = "niri-profiles";
const SETTINGS_DIR: &str = "niri-profiles";
const SETTINGS_FILE: &str = "profiles_dir";

// ───────────────────────────── Пути ─────────────────────────────

struct Paths {
    /// ~/.config/niri — здесь живёт config.kdl и симлинк current-profile.kdl
    niri_dir: PathBuf,
    /// Папка, где лежат профили (по умолчанию niri_dir/profiles, можно выбрать свою)
    profiles_dir: PathBuf,
    link: PathBuf,
    config: PathBuf,
}

/// $XDG_CONFIG_HOME или ~/.config
fn config_home() -> Result<PathBuf, String> {
    match env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(xdg) => Ok(PathBuf::from(xdg)),
        None => {
            let home = env::var_os("HOME").ok_or("Переменная окружения HOME не задана")?;
            Ok(PathBuf::from(home).join(".config"))
        }
    }
}

fn settings_path() -> Result<PathBuf, String> {
    Ok(config_home()?.join(SETTINGS_DIR).join(SETTINGS_FILE))
}

/// Ранее выбранная пользователем папка с профилями.
fn load_custom_dir() -> Option<PathBuf> {
    let raw = fs::read_to_string(settings_path().ok()?).ok()?;
    let line = raw.trim();
    if line.is_empty() { None } else { Some(PathBuf::from(line)) }
}

fn save_custom_dir(dir: &Path) -> Result<(), String> {
    let file = settings_path()?;
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Не удалось создать {}: {e}", parent.display()))?;
    }
    fs::write(&file, dir.to_string_lossy().as_bytes())
        .map_err(|e| format!("Не удалось сохранить настройки {}: {e}", file.display()))
}

/// Если выбрана папка с подпапкой `profiles/` — берём её, иначе саму папку.
fn resolve_profiles_dir(picked: &Path) -> PathBuf {
    let sub = picked.join(PROFILES_SUBDIR);
    if sub.is_dir() { sub } else { picked.to_path_buf() }
}

impl Paths {
    fn discover() -> Result<Self, String> {
        let niri_dir = config_home()?.join("niri");
        let default_dir = niri_dir.join(PROFILES_SUBDIR);
        let profiles_dir = load_custom_dir()
            .filter(|d| d.is_dir())
            .unwrap_or(default_dir);
        Ok(Self {
            link: niri_dir.join(LINK_NAME),
            config: niri_dir.join(MAIN_CONFIG),
            profiles_dir,
            niri_dir,
        })
    }
}

// ───────────────────────── Файловая логика ─────────────────────────

/// Создаёт пустую папку профилей, если её нет. Никаких заглушек.
fn ensure_profiles_dir(dir: &Path) -> Result<(), String> {
    if dir.exists() {
        return Ok(());
    }
    fs::create_dir_all(dir).map_err(|e| format!("Не удалось создать {}: {e}", dir.display()))
}

/// Имена (без расширения) всех *.kdl в папке профилей, по алфавиту.
/// `current-profile.kdl` никогда не считается профилем; `config.kdl` — тоже,
/// если сканируется сама папка ~/.config/niri (иначе получится цикл include).
fn scan_profiles(dir: &Path, niri_dir: &Path) -> Result<Vec<String>, String> {
    let exclude_config = dir == niri_dir;
    let entries = fs::read_dir(dir)
        .map_err(|e| format!("Не удалось прочитать {}: {e}", dir.display()))?;

    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("kdl") && p.is_file())
        .filter(|p| {
            let file = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            file != LINK_NAME && !(exclude_config && file == MAIN_CONFIG)
        })
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect();

    names.sort_by_key(|n| n.to_lowercase());
    Ok(names)
}

/// Атомарно (tmp-симлинк + rename) направляет `link` на `target`.
fn replace_link(link: &Path, target: &Path) -> Result<(), String> {
    let tmp = link.with_extension(format!("kdl.tmp{}", std::process::id()));
    let _ = fs::remove_file(&tmp);

    symlink(target, &tmp)
        .map_err(|e| format!("Не удалось создать симлинк {}: {e}", tmp.display()))?;

    if let Err(e) = fs::rename(&tmp, link) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("Не удалось обновить {}: {e}", link.display()));
    }
    Ok(())
}

fn switch_symlink(paths: &Paths, name: &str) -> Result<(), String> {
    let target = paths.profiles_dir.join(format!("{name}.kdl"));
    if !target.is_file() {
        return Err(format!("Файл профиля не найден: {}", target.display()));
    }

    // Не затираем обычный файл — сначала его нужно сохранить кнопкой «Бэкап»
    if let Ok(meta) = fs::symlink_metadata(&paths.link) {
        if !meta.file_type().is_symlink() {
            return Err(format!(
                "{} — обычный файл, а не симлинк. Нажмите «Бэкап», чтобы сохранить его как профиль",
                paths.link.display()
            ));
        }
    }
    replace_link(&paths.link, &target)
}

// ───────────────────── Обнаружение и бэкап текущего конфига ─────────────────────

/// Какой файл сейчас считается «текущим конфигом пользователя».
/// 1) current-profile.kdl (симлинк или обычный файл);
/// 2) config.kdl — но только если он сам не подключает current-profile.kdl.
fn current_source(paths: &Paths) -> Option<PathBuf> {
    if paths.link.is_file() {
        return Some(paths.link.clone());
    }
    if paths.config.is_file() {
        let includes_link = fs::read_to_string(&paths.config)
            .map(|s| s.contains(LINK_NAME))
            .unwrap_or(false);
        if !includes_link {
            return Some(paths.config.clone());
        }
    }
    None
}

fn timestamp() -> String {
    Command::new("date")
        .arg("+%Y%m%d-%H%M%S")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs().to_string())
                .unwrap_or_else(|_| "0".into())
        })
}

/// Безопасное имя файла из пользовательского ввода.
fn sanitize_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let trimmed = trimmed
        .strip_suffix(".kdl")
        .or_else(|| trimmed.strip_suffix(".KDL"))
        .unwrap_or(trimmed);

    let cleaned: String = trimmed
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | ' ' | '.') { c } else { '_' })
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').trim().to_string();

    if cleaned.is_empty() { None } else { Some(cleaned) }
}

/// name, name-2, name-3… — чтобы не затереть существующий профиль.
fn unique_name(dir: &Path, base: &str) -> String {
    let mut name = base.to_string();
    let mut n = 2;
    while dir.join(format!("{name}.kdl")).exists() {
        name = format!("{base}-{n}");
        n += 1;
    }
    name
}

/// Сохраняет текущий конфиг как новый профиль в папке профилей.
/// Если current-profile.kdl был обычным файлом, он заменяется симлинком
/// на сохранённую копию (содержимое не теряется, профиль становится активным).
fn backup_current(paths: &Paths, requested: &str) -> Result<String, String> {
    let source = current_source(paths).ok_or_else(|| {
        format!(
            "Нечего сохранять: нет {}, а {} отсутствует или сам подключает профиль",
            LINK_NAME, MAIN_CONFIG
        )
    })?;

    let content = fs::read(&source)
        .map_err(|e| format!("Не удалось прочитать {}: {e}", source.display()))?;

    fs::create_dir_all(&paths.profiles_dir)
        .map_err(|e| format!("Не удалось создать {}: {e}", paths.profiles_dir.display()))?;

    let base = sanitize_name(requested).unwrap_or_else(|| format!("backup-{}", timestamp()));
    let name = unique_name(&paths.profiles_dir, &base);

    let dest = paths.profiles_dir.join(format!("{name}.kdl"));
    let tmp = paths.profiles_dir.join(format!(".{name}.tmp"));

    fs::write(&tmp, &content)
        .map_err(|e| format!("Не удалось записать {}: {e}", tmp.display()))?;
    if let Err(e) = fs::rename(&tmp, &dest) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("Не удалось сохранить {}: {e}", dest.display()));
    }

    let link_is_plain_file = fs::symlink_metadata(&paths.link)
        .map(|m| m.file_type().is_file())
        .unwrap_or(false);
    if source == paths.link && link_is_plain_file {
        replace_link(&paths.link, &dest)?;
    }

    Ok(name)
}

// ──────────────────────── Внешние команды ────────────────────────

fn run(cmd: &str, args: &[&str]) -> Result<(), String> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("Не удалось запустить `{cmd}`: {e}"))?;

    if out.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        Err(format!("`{cmd}` завершилась с ошибкой: {}", stderr.trim()))
    }
}

fn reload_niri() -> Result<(), String> {
    run("niri", &["msg", "action", "reload-config"])
}

/// Ошибка уведомления не критична — только логируем.
fn notify(body: &str) {
    if let Err(e) = run("notify-send", &["Niri", body]) {
        eprintln!("[warn] {e}");
    }
}

fn apply_profile(paths: &Paths, name: &str) -> Result<String, String> {
    switch_symlink(paths, name)?;
    reload_niri().map_err(|e| format!("Профиль «{name}» выбран, но перезагрузка не удалась: {e}"))?;
    notify(&format!("Профиль {name} успешно применен"));
    Ok(format!("Профиль «{name}» применён"))
}

// ───────────────────────────── UI ─────────────────────────────

fn show_status(ui: &AppWindow, msg: String, is_error: bool) {
    ui.set_status_text(msg.into());
    ui.set_status_error(is_error);
}

fn refresh(ui: &AppWindow, paths: &Paths) {
    ui.set_profiles_path(paths.profiles_dir.display().to_string().into());

    let names = match scan_profiles(&paths.profiles_dir, &paths.niri_dir) {
        Ok(n) => n,
        Err(e) => {
            show_status(ui, e, true);
            Vec::new()
        }
    };

    // Активный профиль определяем по реальному файлу, на который указывает симлинк
    let active = fs::canonicalize(&paths.link).ok();

    let items: Vec<ProfileItem> = names
        .into_iter()
        .map(|name| {
            let file = paths.profiles_dir.join(format!("{name}.kdl"));
            let is_active = active.is_some() && fs::canonicalize(&file).ok() == active;
            let initial = name
                .chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_default();
            ProfileItem {
                initial: initial.into(),
                active: is_active,
                name: name.into(),
            }
        })
        .collect();

    let empty = items.is_empty();
    ui.set_profiles(ModelRc::new(VecModel::from(items)));

    let detected = if empty { current_source(paths) } else { None };
    ui.set_has_detected_config(detected.is_some());
    ui.set_detected_text(
        detected
            .map(|p| p.display().to_string())
            .unwrap_or_default()
            .into(),
    );
}

fn main() -> Result<(), slint::PlatformError> {
    let _ = slint::set_xdg_app_id(APP_ID);

    // Запасной способ выбрать папку без диалога: niri-profiles --dir /путь/к/профилям
    let args: Vec<String> = env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--dir") {
        match args.get(i + 1) {
            Some(d) => {
                let dir = resolve_profiles_dir(Path::new(d));
                if let Err(e) = save_custom_dir(&dir) {
                    eprintln!("[error] {e}");
                }
            }
            None => eprintln!("[error] после --dir нужен путь"),
        }
    }

    let ui = AppWindow::new()?;

    let paths = match Paths::discover() {
        Ok(p) => Rc::new(RefCell::new(p)),
        Err(e) => {
            show_status(&ui, e, true);
            return ui.run();
        }
    };

    if let Err(e) = ensure_profiles_dir(&paths.borrow().profiles_dir) {
        eprintln!("[error] {e}");
        show_status(&ui, e, true);
    }

    refresh(&ui, &paths.borrow());

    // Клик по карточке профиля
    {
        let weak = ui.as_weak();
        let paths = Rc::clone(&paths);
        ui.on_select_profile(move |name| {
            let Some(ui) = weak.upgrade() else { return };
            let p = paths.borrow();
            match apply_profile(&p, &name) {
                Ok(msg) => show_status(&ui, msg, false),
                Err(e) => {
                    eprintln!("[error] {e}");
                    show_status(&ui, e, true);
                }
            }
            refresh(&ui, &p);
        });
    }

    // Бэкап текущего профиля (кнопка «Бэкап»)
    {
        let weak = ui.as_weak();
        let paths = Rc::clone(&paths);
        ui.on_backup_current(move |name| {
            let Some(ui) = weak.upgrade() else { return };
            let p = paths.borrow();
            match backup_current(&p, &name) {
                Ok(saved) => {
                    notify(&format!("Текущий профиль сохранён как {saved}"));
                    show_status(&ui, format!("Сохранено как «{saved}»"), false);
                    ui.set_backup_name("".into());
                }
                Err(e) => {
                    eprintln!("[error] {e}");
                    show_status(&ui, e, true);
                }
            }
            refresh(&ui, &p);
        });
    }

    // Импорт найденного конфига из пустого состояния
    {
        let weak = ui.as_weak();
        let paths = Rc::clone(&paths);
        ui.on_import_current(move || {
            let Some(ui) = weak.upgrade() else { return };
            let p = paths.borrow();
            match backup_current(&p, "current") {
                Ok(saved) => show_status(&ui, format!("Сохранено как «{saved}»"), false),
                Err(e) => {
                    eprintln!("[error] {e}");
                    show_status(&ui, e, true);
                }
            }
            refresh(&ui, &p);
        });
    }

    // Выбор папки: диалог открываем в отдельном потоке, чтобы не вешать UI
    {
        let weak = ui.as_weak();
        ui.on_choose_folder(move || {
            let weak = weak.clone();
            std::thread::spawn(move || {
                let picked = rfd::FileDialog::new()
                    .set_title("Папка с .kdl конфигами")
                    .pick_folder();
                if let Some(dir) = picked {
                    let s = dir.to_string_lossy().into_owned();
                    let _ = weak.upgrade_in_event_loop(move |ui| ui.invoke_folder_picked(s.into()));
                }
            });
        });
    }

    // Папка выбрана
    {
        let weak = ui.as_weak();
        let paths = Rc::clone(&paths);
        ui.on_folder_picked(move |path| {
            let Some(ui) = weak.upgrade() else { return };
            let dir = resolve_profiles_dir(Path::new(path.as_str()));

            let found = scan_profiles(&dir, &paths.borrow().niri_dir);
            match found {
                Ok(list) if !list.is_empty() => {
                    if let Err(e) = save_custom_dir(&dir) {
                        eprintln!("[warn] {e}");
                    }
                    paths.borrow_mut().profiles_dir = dir.clone();
                    show_status(
                        &ui,
                        format!("Папка профилей: {} (найдено: {})", dir.display(), list.len()),
                        false,
                    );
                }
                Ok(_) => show_status(
                    &ui,
                    format!("В {} нет .kdl файлов — папка не изменена", dir.display()),
                    true,
                ),
                Err(e) => show_status(&ui, e, true),
            }
            refresh(&ui, &paths.borrow());
        });
    }

    // Кнопка Force Reload
    {
        let weak = ui.as_weak();
        let paths = Rc::clone(&paths);
        ui.on_force_reload(move || {
            let Some(ui) = weak.upgrade() else { return };
            match reload_niri() {
                Ok(()) => {
                    notify("Конфигурация перезагружена");
                    show_status(&ui, "Конфигурация перезагружена".into(), false);
                }
                Err(e) => {
                    eprintln!("[error] {e}");
                    show_status(&ui, e, true);
                }
            }
            refresh(&ui, &paths.borrow());
        });
    }

    ui.run()
}
