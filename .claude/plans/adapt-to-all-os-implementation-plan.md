Ниже представлен чёткий, пошаговый **план реализации единого кроссплатформенного кода для Windows 10/11, ALT Linux и Ubuntu** в полном соответствии с нашей новой спецификацией v2.0.

---

## 1. Архитектурная цель

Сделать так, чтобы **одна и та же кодовая база** без единой C/C++ библиотеки разработки (`*-devel` / `*-dev`) собиралась стандартной командой `cargo build --release` или `cargo install --path .\src --force` на **Windows, ALT Linux и Ubuntu** и предоставляла идентичный функционал (GUI + CLI + 3-уровневый декодер PDF + кэш + OCR).

---

## 2. Пофайловый план изменений (4 файла)

| Файл | Что меняем | Зачем это нужно |
| :--- | :--- | :--- |
| **`src/Cargo.toml`** | Удаляем старые неиспользуемые C-зависимости (`gtk4`, `libadwaita`, `gio`, `relm4`, `ashpd`, `leptess`, `url`). Делаем фичи `windows-desktop` и `desktop` совместимыми алиасами без внешних крейтов. | Сборка на ALT Linux и Ubuntu больше не потребует установки `libgtk4-devel`, `libadwaita-devel` и `leptonica-devel`. |
| **`src/build.rs`** | Убираем вызовы `glib-compile-resources`, `glib-compile-schemas` и `panic!` при не-Linux сборке. | Скрипт сборки мгновенно проходит на любой ОС без внешних утилит GLib. |
| **`src/src/model/extractor.rs`** | Объединяем ветку OCR (`tesseract` + `pdftoppm`) для Windows и Linux, сохраняя WinRT-фоллбэк для Windows. | На ALT Linux и Ubuntu OCR сразу подхватывает системные `/usr/bin/tesseract` и `/usr/bin/pdftoppm` (`eng+rus` → `rus` → `eng`). |
| **`src/src/app_win.rs` + `src/src/main.rs`** | Заменяем заглушку `#[cfg(not(target_os = "windows"))]` на полноценный Linux-адаптер (`zenity`/`kdialog`, `xdg-open`, `~/.config`, запуск браузера в `--app` режиме) + добавляем возможность быстрого ввода пути папки прямо в UI. | Графическое окно, выбор папки, сохранение настроек и открытие файлов работают нативно и на Windows, и на ALT Linux / Ubuntu. |

---

## 3. Детальный план по подсистемам

### Этап 1. Очистка `src/Cargo.toml` и `src/build.rs`
1. В `src/Cargo.toml` оставляем только реально работающие чистые Rust-зависимости:
   * `tokio`, `tokio-util` (асинхронный пул `JoinSet`, `Semaphore`, `CancellationToken`);
   * `anyhow`, `lopdf`, `pdf-extract`, `quick-xml`, `walkdir`, `zip` (3-уровневый парсинг `.pdf` и `.docx`);
   * `tracing`, `tracing-subscriber` (диагностика).
2. В секции `[features]` прописываем:
   ```toml
   [features]
   default = ["desktop-ui", "ocr"]
   desktop-ui = []
   ocr = []
   windows-ocr = ["ocr"]
   windows-desktop = ["desktop-ui", "ocr"]
   desktop = ["desktop-ui", "ocr"]
   ```
   *Что это даёт:* Любая команда — `cargo run`, `cargo install --path .\src`, `cargo run --features windows-desktop` или `cargo run --features desktop` — будет безошибочно работать на всех трёх ОС.
3. В `src/build.rs` оставляем минимальный `fn main() {}` без вызовов утилит GLib.

---

### Этап 2. Платформенный адаптер для ALT Linux и Ubuntu в `app_win.rs`
Вместо пустой заглушки для не-Windows систем реализуем модуль `platform` с двумя ветками (`#[cfg(target_os = "windows")]` и `#[cfg(not(target_os = "windows"))]`), покрывающий 5 системных функций:

1. **Подавление отладочного спама `stdout` от `pdf-extract` (`silence_stdout_spam`):**
   * **Windows:** перенаправление `STD_OUTPUT_HANDLE` в `NUL` через `CreateFileW` + `SetStdHandle`.
   * **ALT Linux / Ubuntu:** перенаправление файлового дескриптора `STDOUT_FILENO (1)` в `/dev/null` при старте GUI (логи `tracing` при этом идут в `stderr`, а консольные команды `scan`/`search` работают до вызова GUI и штатно печатают результат в терминал).
2. **Путь к файлу настроек (`settings_file_path`):**
   * **Windows:** `%APPDATA%\DocumentSearcher\settings.conf`.
   * **ALT Linux / Ubuntu:** `$XDG_CONFIG_HOME/DocumentSearcher/settings.conf` или `~/.config/DocumentSearcher/settings.conf` (`$HOME/.config/...`).
3. **Системный диалог выбора папки (`pick_folder`):**
   * **Windows:** нативный Win32 COM `SHBrowseForFolderW`.
   * **ALT Linux / Ubuntu:** автоматический перебор установленных в системе графических диалогов:
     1. `zenity --file-selection --directory --title="Select a folder to search" --filename="<dir>/"` (стандарт GNOME / MATE / Xfce / ALT Workstation);
     2. `kdialog --getexistingdirectory "<dir>" --title "Select a folder to search"` (стандарт KDE Plasma / ALT KWorkstation);
     3. `yad --file --directory --title="Select a folder to search"`.
   * **Бонус в интерфейсе (Ручной ввод пути):** Рядом с кнопкой `"Change folder"` добавим возможность вставить или отредактировать путь к папке текстом прямо в окне программы (эндпоинт `POST /api/set-folder`). Это очень удобно и на Windows (когда нужно быстро вставить скопированный путь из Проводника), и на Linux (даже если `zenity` ещё не установлен).
4. **Открытие найденного документа по клику (`open_with_default_app`):**
   * **Windows:** нативный `ShellExecuteW(..., "open", ...)`.
   * **ALT Linux / Ubuntu:** запуск `xdg-open <путь>` (с автоматическим откатом на `gio open <путь>`), который открывает `.docx` в LibreOffice Writer, а `.pdf` — в системном просмотрщике (Evince / Okular).
5. **Запуск графического окна (`find_app_browser` & `spawn_desktop_window`):**
   * **Windows:** поиск `msedge.exe`, `chrome.exe`, `browser.exe` (Yandex) и запуск в режиме отдельного окна `--app=http://127.0.0.1:<port> --window-size=1200,800`.
   * **ALT Linux / Ubuntu:** поиск в `/usr/bin`, `/usr/local/bin`, `/snap/bin` и `PATH` браузеров `chromium`, `chromium-browser`, `google-chrome`, `google-chrome-stable`, `yandex-browser`, `brave-browser`, `microsoft-edge` для запуска в режиме `--app=...`, а если установлен только Firefox — запуск `firefox --new-window <url>` или `xdg-open <url>`.

---

### Этап 3. Кроссплатформенный OCR в `src/src/model/extractor.rs`
1. Объединяем условную компиляцию OCR под единым флагом `#[cfg(any(feature = "ocr", feature = "windows-ocr"))]`:
   * **Поиск `tesseract`:** проверяет переменную `DOCUMENT_SEARCHER_TESSERACT_BIN`, на Windows — `C:\Program Files\Tesseract-OCR\tesseract.exe` и `%LOCALAPPDATA%`, на Linux — `/usr/bin/tesseract`, `/usr/local/bin/tesseract` и `PATH`.
   * **Поиск `pdftoppm`:** проверяет `DOCUMENT_SEARCHER_PDFTOPPM_BIN`, на Windows — `C:\Program Files\poppler\...`, на Linux — `/usr/bin/pdftoppm`, `/usr/local/bin/pdftoppm` и `PATH`.
   * **Автовыбор языков OCR:** по умолчанию запрашивает `eng+rus`; если один из языковых пакетов не установлен, автоматически пробует `rus`, затем `eng`.
   * **Windows WinRT Fallback:** на Windows сохраняется резервный механизм через системный `Windows.Data.Pdf` + `Windows.Media.Ocr`, если внешние `pdftoppm`/`tesseract` не установлены.

---

### Этап 4. Верификация сборки и тестов
После внесения изменений мы проверяем:
1. `cargo fmt --check` — идеальное форматирование.
2. `cargo test` — прохождение всех unit-тестов ядра и UI-контроллера (включая тесты нормализации путей, декодирования кириллицы `uni04XX`, белого списка файлов, пагинации и защиты состояния).
3. `cargo clippy --all-targets -- -D warnings` — полное отсутствие предупреждений компилятора и линтера.
4. `cargo install --path .\src --force` — установка универсального бинарника.

---

Если план полностью устраивает — дай команду, и я выдам готовый код обновлённых файлов (`Cargo.toml`, `build.rs`, `extractor.rs`, `main.rs`, `app_win.rs`)!