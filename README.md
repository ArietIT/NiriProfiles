# niri-profiles

<p align="left">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white">
  <img alt="Slint" src="https://img.shields.io/badge/Slint-2379F4?style=for-the-badge">
  <img alt="Wayland" src="https://img.shields.io/badge/Wayland-FFBC00?style=for-the-badge&logo=wayland&logoColor=black">
  <img alt="License" src="https://img.shields.io/badge/License-MIT-6750A4?style=for-the-badge">
</p>

Лёгкое GUI-приложение в стиле **Material You** для быстрого переключения профилей конфигурации Wayland-композитора [Niri](https://github.com/YaLTeR/niri).

Один клик по карточке — и симлинк `current-profile.kdl` атомарно переключается на выбранный профиль, а Niri перечитывает конфиг.

<!-- Добавьте скриншот: положите его в docs/screenshot.png и раскомментируйте строку ниже -->
<!-- ![screenshot](docs/screenshot.png) -->

## Возможности

- 🎨 Интерфейс Material 3: пастельные тона, крупные скругления, плавные анимации при наведении и нажатии, светлая/тёмная тема по системной.
- 🔀 Атомарное переключение профилей (временный симлинк + `rename()`), Niri никогда не видит «битую» ссылку.
- 🔍 Приложение само находит ваш текущий конфиг и предлагает сохранить его как профиль. Никаких шаблонов и заглушек.
- 💾 Кнопка **«Бэкап»** сохраняет текущий конфиг как новый профиль, чтобы при установке чужих dotfiles (например, MacShell) оба профиля остались в списке.
- 📁 Если профили не видны, можно **выбрать папку с конфигами** вручную; выбор запоминается.
- 🔄 Кнопка **Force Reload** принудительно вызывает `niri msg action reload-config`.
- 🔔 Уведомления через `notify-send`.

## Как это работает

```
~/.config/niri/
├── config.kdl              # основной конфиг, в нём: include "current-profile.kdl"
├── current-profile.kdl  -> profiles/<выбранный>.kdl   (симлинк)
└── profiles/
    ├── work.kdl
    └── home.kdl
```

1. Приложение сканирует папку профилей и показывает все `*.kdl` карточками.
2. По клику обновляет симлинк `current-profile.kdl`.
3. Выполняет `niri msg action reload-config` и отправляет уведомление.

Папка по умолчанию: `$XDG_CONFIG_HOME/niri/profiles` (обычно `~/.config/niri/profiles`). При первом запуске она создаётся пустой.

### Бэкап текущего профиля

Нажмите **«Бэкап»** (имя можно задать в поле слева, иначе будет `backup-ГГГГММДД-ЧЧММСС`).
Источником копии служит `current-profile.kdl`; если его нет, берётся `config.kdl` (при условии, что он сам не подключает `current-profile.kdl`).

Если `current-profile.kdl` был **обычным файлом** (например, его создал установщик чужого конфига), после бэкапа он заменяется симлинком на сохранённую копию. Содержимое не теряется, профиль сразу становится активным.

Существующие профили никогда не перезаписываются: при совпадении имени добавляется `-2`, `-3` и т. д.

### Выбор своей папки

Кнопка с иконкой папки в заголовке открывает системный диалог выбора папки (через `xdg-desktop-portal`). Если внутри выбранной папки есть подпапка `profiles/`, используется она. Выбор сохраняется в `~/.config/niri-profiles/profiles_dir`.

Если диалог недоступен (нет портала), папку можно задать из терминала:

```bash
niri-profiles --dir ~/dotfiles/niri/profiles
```

## Установка

### Зависимости

- Rust (stable) и `cargo`
- `fontconfig`, `libxkbcommon` (на Arch-based обычно уже есть)
- `libnotify` и демон уведомлений (mako, dunst, swaync…) для `notify-send`
- `xdg-desktop-portal` + `xdg-desktop-portal-gnome` или `-gtk` для диалога выбора папки

```bash
# Arch / EndeavourOS / CachyOS
sudo pacman -S --needed rust fontconfig libxkbcommon libnotify xdg-desktop-portal-gtk
```

### Сборка

```bash
git clone https://github.com/ArietIT/niri-profiles.git
cd niri-profiles
cargo build --release
install -Dm755 target/release/niri-profiles ~/.local/bin/niri-profiles
```

## Настройка Niri

В `~/.config/niri/config.kdl` подключите симлинк (директива `include` доступна в новых версиях Niri, проверьте `niri --version`):

```kdl
include "current-profile.kdl"
```

Чтобы окно открывалось плавающим, а не тайлом:

```kdl
window-rule {
    match app-id="niri-profiles"
    open-floating true
}
```

Запуск по горячей клавише:

```kdl
binds {
    Mod+Shift+P { spawn "niri-profiles"; }
}
```

## Ярлык в меню приложений (по желанию)

`~/.local/share/applications/niri-profiles.desktop`:

```ini
[Desktop Entry]
Type=Application
Name=Niri Profiles
Comment=Переключение профилей конфигурации Niri
Exec=niri-profiles
Terminal=false
Categories=Utility;Settings;
```

## Структура проекта

```
niri-profiles/
├── Cargo.toml
├── build.rs
├── ui/
│   └── appwindow.slint     # интерфейс Material 3
└── src/
    └── main.rs             # файловая логика, вызов niri и notify-send
```

## Лицензия

[MIT](LICENSE)
