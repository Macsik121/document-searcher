# Document Searcher

**Document Searcher** — высокопроизводительное настольное (GUI) и консольное (CLI) приложение на языке Rust для глубокого контекстного поиска текста внутри локальных документов **`.docx`** и **`.pdf`** на платформах **Windows 10/11**, **ALT Linux** и **Ubuntu**.

---

## Основные возможности

* **Фокус на документах `.docx` и `.pdf`:**
  * По умолчанию сканер индексирует только файлы `.docx` и `.pdf` (регистронезависимо), игнорируя исходный код, временные и скрытые файлы (начинающиеся с `.`), символические ссылки и файлы размером более **50 MiB**.
  * При необходимости список форматов можно расширить через параметр `--ext` (например, `--ext docx,pdf,txt,md`).
* **Трёхуровневый конвейер извлечения текста из PDF (`3-Tier PDF Extractor`):**
  1. **Постраничный декодер `lopdf` с поддержкой кириллицы:** напрямую разбирает шрифтовые таблицы `/Differences` (глифы `/uni0410`..`/uni044F`, `/afii10017`..`/afii10097`, `.cap`), карты символов `/ToUnicode` (`bfchar` / `bfrange`), `UTF-16BE` и кодировку `CP1251`.
  2. **Изолированный парсер `pdf-extract`:** выполняется внутри защитного контура `std::panic::catch_unwind`, благодаря чему сложные или повреждённые PDF-файлы никогда не прерывают общий поиск.
  3. **Интеллектуальный OCR-фоллбэк для сканов:** если PDF не содержит текстового слоя, автоматически запускается распознавание через `pdftoppm` + `tesseract` (`eng+rus` с откатом на `eng`), а на Windows 10/11 при отсутствии внешних утилит используется встроенный системный движок **Windows WinRT (`Windows.Data.Pdf` + `Windows.Media.Ocr`)**.
* **Мгновенный повторный поиск (`In-Memory TEXT_CACHE`):**
  * Извлечённый текст каждого файла кэшируется в оперативной памяти по ключу `(путь, размер, время модификации)`. Повторные поисковые запросы по проиндексированной папке выполняются за миллисекунды без повторного чтения диска.
* **Многопоточность на всех ядрах CPU:**
  * Извлечение текста и поиск выполняются параллельно в пуле `tokio::task::JoinSet`, ограниченном семафором по числу ядер процессора (`available_parallelism`, от 2 до 8 потоков).
  * Защита от «keystroke bomb»: задержка ввода (**debounce 300 мс**), мгновенная отмена устаревшего поиска через `CancellationToken` и строгая сериализация команд через монитор `Controller`.
* **Unicode-поиск с контекстом (±20 символов):**
  * Регистронезависимый поиск с сохранением точных байтовых границ символов UTF-8.
  * Для каждого совпадения формируется сниппет: **до 20 символов до** + **точная найденная подстрока (выделяется маркером)** + **до 20 символов после**.
* **Два режима работы в одном исполняемом файле (GUI + CLI):**
  * **Графический интерфейс (GUI):** единый кроссплатформенный движок — встроенный HTTP-сервер (`127.0.0.1`) + окно системного браузера в режиме `--app`. Разделение экрана 50/50, список найденных файлов слева, строка поиска с индикацией прогресса (спиннер / галочка успеха / ошибка), карточки результатов с подсветкой совпадений, кумулятивная пагинация по **100 результатов** (`Show more`), **поле быстрого ввода пути папки** (кнопка `Go`) рядом с системным диалогом выбора папки, и открытие документов по клику в программе по умолчанию.
  * **Консольный интерфейс (CLI):** быстрые команды `scan` и `search` для работы в терминале и скриптах.

---

## Быстрый старт и развёртывание (Windows 10/11)

### 1. Предварительные требования
* Установленный тулчейн **Rust** (`rustup`, стабильная версия, см. `rust-toolchain.toml`).
* Никаких дополнительных C/C++ библиотек на Windows устанавливать не требуется.
* Для OCR отсканированных PDF (опционально): внешние `tesseract.exe` + `pdftoppm.exe` (Poppler). Если они не установлены, на Windows 10/11 автоматически срабатывает резервный системный движок **Windows WinRT OCR**.

### 2. Настройка корневого Workspace (выполняется один раз)
Если вы запускаете команды из корня репозитория (над папкой `src`), убедитесь, что в корне лежит файл `Cargo.toml` со следующим содержимым:

```toml
[workspace]
members = ["src"]
default-members = ["src"]
resolver = "2"
```

### 3. Глобальная установка для запуска из любой папки
Выполните в PowerShell из корня репозитория:

```powershell
# Полная установка: GUI + CLI + OCR (Windows WinRT OCR как резерв)
cargo install --path .\src --features windows-desktop --force

# Только CLI (без графического интерфейса) — самый лёгкий бинарник
cargo install --path .\src --no-default-features --force
```

Команда с `--features windows-desktop` скомпилирует релизный бинарник `document-searcher.exe` и установит его в `%USERPROFILE%\.cargo\bin`, после чего приложение можно вызывать из **любой** директории на компьютере.

### 4. Сборка и запуск через Cargo (без глобальной установки)

```powershell
# Сборка релизной версии с GUI
cargo build --release --features windows-desktop

# Запуск графического интерфейса (GUI)
cargo run --release --features windows-desktop

# Запуск GUI сразу для конкретной папки
cargo run --release --features windows-desktop -- "C:\Users\Имя\Documents\Отчёты"

# Запуск консольного сканирования и поиска (CLI)
cargo run --no-default-features -- scan "C:\Users\Имя\Documents\Отчёты"
cargo run --no-default-features -- search "C:\Users\Имя\Documents\Отчёты" "искомая строка"
```

---

## Использование установленного приложения

После выполнения `cargo install` откройте терминал в любой нужной папке с документами:

```powershell
# 1. Открыть графическое окно поиска для текущей папки
document-searcher .

# 2. Открыть графическое окно поиска для указанной папки
document-searcher "D:\Документы\Договоры"

# 3. То же через явную подкоманду gui
document-searcher gui "D:\Документы\Договоры"

# 4. Открыть графическое окно с последней сохранённой папкой
document-searcher

# 5. Просканировать папку в консоли (по умолчанию только .docx и .pdf)
document-searcher scan .

# 6. Найти подстроку во всех .docx и .pdf файлах папки в консоли
document-searcher search . "финансовый отчёт"

# 7. Запустить поиск с пользовательским списком расширений (--ext)
document-searcher search . "config" --ext docx,pdf,txt,md

# 8. Краткая справка по командам
document-searcher --help
```

Внутри GUI рядом с кнопкой **`Change folder`** есть поле ввода — можно вставить или отредактировать путь к папке текстом и нажать **`Go`** (или `Enter`). Это удобно, когда нужно быстро вставить скопированный путь из Проводника, или если на Linux ещё не установлен `zenity`/`kdialog`.

---

## Развёртывание на ALT Linux и Ubuntu

На Linux используется **тот же кроссплатформенный модуль `app_win.rs`**, что и на Windows: встроенный HTTP-сервер + окно браузера в режиме `--app`. Системные диалоги выбора папки выполняются через `zenity` → `kdialog` → `yad`, файлы открываются через `xdg-open` (с откатом на `gio open`), а графическое окно запускается в `chromium`/`google-chrome`/`yandex-browser`/`brave-browser`/`microsoft-edge` (или `firefox --new-window` как резерв).

### 1. Установка системных зависимостей

* **Графический диалог выбора папки (один из):** `zenity` (GNOME/MATE/Xfce), `kdialog` (KDE) или `yad`.
* **Открытие документов:** `xdg-utils` (предоставляет `xdg-open`).
* **OCR отсканированных PDF (опционально):** `tesseract` + `poppler-tools` (pdftoppm).
* **Графическое окно GUI:** любой Chromium-подобный браузер или Firefox.

**На ALT Linux (`apt-rpm`):**
  ```bash
  sudo apt-get update
  sudo apt-get install -y gcc make pkgconfig tesseract tesseract-langpack-ru tesseract-langpack-en poppler-tools zenity xdg-utils
  ```
**На Ubuntu / Debian (`apt`):**
  ```bash
  sudo apt update
  sudo apt install -y build-essential pkg-config tesseract-ocr tesseract-ocr-rus tesseract-ocr-eng poppler-utils zenity xdg-utils
  ```

### 2. Установка Rust (если не установлен)
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
```

### 3. Сборка и установка приложения
```bash
# Полная установка: GUI + CLI + OCR (внешние tesseract/pdftoppm)
cargo install --path ./src --features windows-desktop --force

# Только CLI (без графического интерфейса)
cargo install --path ./src --no-default-features --force

# Запуск в текущей директории
document-searcher .
```

> **Почему `--features windows-desktop` на Linux?** Имя фичи историческое — она включает кроссплатформенный десктопный модуль `app_win.rs`, который теперь един для Windows и Linux. C/C++-зависимостей (`*-devel`) для этой сборки **не требуется**.

*(Примечание: для альтернативного нативного GTK4/Libadwaita интерфейса через `app.rs` и упаковки во Flatpak см. инструкции в [`src/BUILD_INSTRUCTIONS.md`](src/BUILD_INSTRUCTIONS.md) — этот вариант активируется фичей `--features desktop` и требует `libgtk4-devel`/`libadwaita-devel`.)*

---

## Проверка качества кода и тестирование

Перед коммитом или релизом выполните стандартный набор проверок из корня репозитория (или из папки `src/`):

```powershell
# 1. Проверка форматирования кода
cargo fmt --check

# 2. Запуск unit-тестов ядра (scanner, extractor, search, worker, state)
cargo test --lib --no-default-features

# 3. Запуск unit-тестов десктопного контроллера и сериализации
cargo test --bins --features windows-desktop

# 4. Статический анализ Clippy (без единого предупреждения)
cargo clippy --lib --tests --no-default-features -- -D warnings
cargo clippy --all-targets --features windows-desktop -- -D warnings
```

---

## Сборочные фичи (Cargo features)

| Фича | Что включает | Зависимости |
| :--- | :--- | :--- |
| *(нет фич, `--no-default-features`)* | Только CLI (`scan`, `search`) + ядро поиска | Чистый Rust, без C-deps |
| `windows-desktop` | CLI + **кроссплатформенный GUI** (`app_win.rs`) + Windows WinRT OCR + OCR через внешние `tesseract`/`pdftoppm` | Чистый Rust, без C-deps. Работает на **Windows и Linux** |
| `desktop` | CLI + альтернативный нативный GUI на GTK4/Libadwaita (`app.rs`) + OCR | Требует `libgtk4-devel`, `libadwaita-devel`, `leptonica-devel` |
| `ocr` / `windows-ocr` | Только OCR-подсистему (внешние `tesseract` + `pdftoppm`), без GUI | Чистый Rust |

> При сборке с обеими фичами `desktop` имеет приоритет над `windows-desktop` (см. `src/src/main.rs`).

## Переменные окружения

| Переменная | Назначение |
| :--- | :--- |
| `DOCUMENT_SEARCHER_INITIAL_DIR` | Папка, открываемая GUI при старте (ставится из CLI-аргумента `gui <dir>`). |
| `DOCUMENT_SEARCHER_EXTENSIONS` | Список расширений через запятую (переопределяет `--ext` для последующих запусков). |
| `DOCUMENT_SEARCHER_TESSERACT_BIN` | Явный путь к `tesseract` (иначе авто-поиск в стандартных расположениях ОС). |
| `DOCUMENT_SEARCHER_PDFTOPPM_BIN` | Явный путь к `pdftoppm` (Poppler). |
| `DOCUMENT_SEARCHER_OCR_LANG` | Языки OCR (по умолчанию `eng+rus`; при отсутствии пакета откат `rus` → `eng`). |

---

## Структура проекта и документация

```text
document-searcher/
├── Cargo.toml                         # Корневой Cargo Workspace
├── README.md                          # Основное руководство по проекту
├── docs/
│   ├── document-searcher-documentation.md  # Единая спецификация v2.0 (SRS + BPMN + UML + ERD)
│   └── old-docs-during-the-work/      # Архив документации этапа разработки
└── src/
    ├── Cargo.toml                     # Манифест пакета document-searcher (фичи: windows-desktop, desktop, ocr)
    ├── rust-toolchain.toml            # Конфигурация компилятора (stable channel)
    ├── build.rs                       # Скрипт сборки
    ├── BUILD_INSTRUCTIONS.md          # Подробные инструкции по сборке
    └── src/
        ├── lib.rs                     # Библиотечная точка входа ядра
        ├── main.rs                    # Универсальная точка входа (GUI + CLI)
        ├── app_win.rs                 # Кроссплатформенный десктоп (Win32 + Linux zenity/xdg-open + HTTP-пул + встроенный UI)
        ├── app.rs                     # Альтернативный GTK4/Libadwaita адаптер (фича desktop)
        └── model/
            ├── actions.rs             # Протокол команд и событий (WorkerCommand / WorkerEvent)
            ├── state.rs               # Состояние приложения (AppState) и пагинация
            ├── scanner.rs             # Сканер директорий (.docx, .pdf, лимит 50 MiB)
            ├── extractor.rs           # 3-уровневый экстрактор (lopdf + docx + кроссплатформ. OCR + TEXT_CACHE)
            ├── search.rs              # Регистронезависимый Unicode-поиск с контекстом ±20 симв.
            └── worker.rs              # Многопоточный Tokio-воркер (Debounce 300ms, JoinSet)
```

* **[Единая спецификация и архитектура (BPMN, UML, ERD)](docs/document-searcher-documentation.md)** — полный технический справочник
