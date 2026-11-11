# DEVELOPMENT HANDOFF — document-searcher

> **Назначение документа.** Это полный handoff сессии разработки `document-searcher`,
> перенесённый из диалога, чтобы можно было продолжить работу с **чистой сессии без
> контекста**. Документ фиксирует каждое требование пользователя (дословно), что было
> реализовано по факту, что предложено к разработке, текущее состояние кода и
> известные проблемы.
>
> **Как пользоваться с чистой сессии.**
> 1. Прочитай раздел **0 (Метаданные)** и **9 (Известные проблемы)** — там критическое
>    состояние «что прямо сейчас сломано».
> 2. Прочитай **4 (Таблица «Промпт → реализованные функции»)** — карта что сделано.
> 3. Прочитай **3 (Хронология диалога)** при необходимости восстановить ход мысли.
> 4. Раздел **10 (Backlog)** — что предложено, но не реализовано.
> 5. Полный транскрипт сессии (со всем выводом команд и reasoning) — см. путь в
>    **приложении 12.3**.

---

## 0. Метаданные

| Поле | Значение |
|---|---|
| Дата составления | 2026-11-09 |
| Ветка git | `main` |
| Последний commit | `def1e93` — «Almost working search through .docx and .pdf files. Almost there.» |
| Рабочее дерево | 7 изменённых файлов (`app_win.rs`, `actions.rs`, `extractor.rs`, `scanner.rs`, `search.rs`, `state.rs`, `worker.rs`) + `prompts-in-between-sessions.txt`; дифф `+586 / −247` |
| Untracked-артефакты сессии | `_user_msgs.txt`, `_user_msgs_clean.txt` (временные извлечения промптов — можно удалить) |
| Платформа разработки | Windows 11 Pro 10.0.26200 (win32) |
| Модель сессии | `funpay/glm-5.2` (в сессии были переключения: `funpay/glm-5.2` → `kr/glm-5` → `funpay/deepseek-v4.1-flash` → `funpay/glm-5.2`) |
| Роль (задана пользователем) | Principal Rust Software Architect & Systems Engineer |
| Источник полного контекста | JSONL-транскрипт — см. **приложение 12.3** |

**Текущее «горящее» состояние (читать первым):**

- ❌ **`cargo fmt --check` ПАДАЕТ** — 2 расхождения: `src/model/extractor.rs:242` и
  `src/app_win.rs:1311` (после правок Task H `cargo fmt` не перезапускался).
- ❌ **`cargo clippy --all-targets --features windows-desktop -- -D warnings` ПАДАЕТ** —
  orphaned doc-comment в `src/app_win.rs:515–517` (остаток от удалённой функции
  `build_pdf_page_url`). На сборках без `windows-desktop` clippy чистый.
- ✅ `cargo test --lib --no-default-features` — **24 теста pass**.
- ✅ `cargo test --bins --features windows-desktop` — **5 тестов pass**.
- ✅ `cargo clippy --lib --tests --no-default-features -- -D warnings` — чисто.

> **Важно:** предыдущие заявления в сессии о «fmt clean / clippy clean» устарели —
> они относились к моменту до правок Task H (расширения, нормализация пробелов,
> упрощение `open_file_at_page`). Сейчас `fmt` и `clippy`(windows-desktop) снова красные.

---

## 1. Обзор проекта

**Назначение.** Настольное (GUI) и консольное (CLI) приложение на Rust для глубокого
контекстного поиска текста внутри локальных документов `.docx` и `.pdf` (плюс
расширяемый список текстовых форматов) на Windows 10/11, ALT Linux, Ubuntu.

**Стек.** Rust edition 2021, стабильный тулчейн (`rust-toolchain.toml`).
Основные зависимости: `lopdf 0.31`, `pdf-extract 0.7`, `quick-xml 0.31`, `walkdir 2.4`,
`zip 0.6`, `encoding_rs 0.8` + `chardetng 0.1` (робастное декодирование текста),
`tokio 1.38` (rt-multi-thread, macros, fs, sync, time), `tokio-util 0.7`,
`async-channel 2.3`, `anyhow 1.0`, `tracing 0.1` + `tracing-subscriber 0.3`.
Опциональные (фичи): `leptess 0.14` (OCR), `libadwaita`/`gtk4`/`gio`/`relm4`/`ashpd`/`url`
(GTK-десктоп на Linux).

**Дерево модулей** (`src/src/`, размеры строк):

```
src/src/
├── main.rs            (177)   — точка входа, CLI-парсер (scan/search/gui/--ext), cfg-гейтинг UI
├── lib.rs             (1)     — re-export model
├── app.rs             (701)   — Linux GTK/relm4 UI (фича `desktop`); НЕ компилировался в этой сессии
├── app_win.rs         (2358)  — кросс-платформенный UI (Win32 + Linux CLI-фоллбэки), HTTP-сервер, JS/CSS
└── model/
    ├── state.rs       (179)   — FileEntry, ExtractedPage, SearchOccurrence, SearchMatch, AppState
    ├── actions.rs     (76)    — WorkerCommand, WorkerEvent (вкл. SearchResultFound), AppMsg
    ├── scanner.rs     (189)   — walkdir-обход, фильтр расширений, MAX_FILE_SIZE=50MiB
    ├── extractor.rs   (1457)  — 3-уровневый PDF-экстрактор, DOCX, plain-text, OCR, кэши
    ├── search.rs      (204)   — Unicode-поиск с контекстом ±20 символов, постранично
    └── worker.rs      (486)   — tokio-воркер, JoinSet, streaming результатов, отмена, debounce
```

---

## 2. Сборка, фичи, запуск

### 2.1 Cargo features (`src/Cargo.toml`)

| Фича | Назначение | Зависимости |
|---|---|---|
| `default = []` | Только ядро поиска (CLI). UI не входит. | — |
| `ocr` | OCR через `tesseract`+`pdftoppm` (внешние CLI). | `leptess` |
| `windows-ocr` | OCR через Windows WinRT (`Windows.Data.Pdf` + `Windows.Media.Ocr`), без внешних утилит. | — |
| `desktop` | Linux GTK/relm4 UI (через `app.rs`). Включает `ocr`. | `libadwaita`, `gtk4`, `gio`, `relm4`, `ashpd`, `url` |
| `windows-desktop = ["windows-ocr"]` | Кросс-платформенный UI через `app_win.rs` (Win32 на Windows, zenity/xdg-open/chromium на Linux). **Историческое имя** — модуль кросс-платформенный. | `windows-ocr` |

**Гейтинг UI в `main.rs`:** `desktop` имеет приоритет над `windows-desktop`.
Без обеих фич — только CLI (`scan`/`search`) и `--help`.

### 2.2 CLI-команды (`src/src/main.rs`)

| Команда | Действие |
|---|---|
| `document-searcher [dir]` | Открыть GUI (в `dir` или текущей папке). |
| `document-searcher gui [dir]` | Открыть GUI в указанной папке. |
| `document-searcher scan <dir> [--ext docx,pdf]` | Скан папки в консоль. |
| `document-searcher search <dir> <query> [--ext ...]` | Поиск в консоль. |
| `document-searcher --help` / `-h` | Справка. |

### 2.3 Переменные окружения

| Переменная | Назначение | Где читается |
|---|---|---|
| `DOCUMENT_SEARCHER_EXTENSIONS` | Список расширений через запятую (переопределяет дефолт `docx,pdf`). | `scanner.rs:52` (`scan_directory_with_report`), `main.rs:94` (`--ext`) |
| `DOCUMENT_SEARCHER_INITIAL_DIR` | Стартовая папка GUI. | `app_win.rs` (init), `main.rs:134,158` |
| `DOCUMENT_SEARCHER_OCR_LANG` | Язык OCR (по умолчанию `eng+rus` → `eng`). | OCR-функции extractor.rs |
| `TESSERACT_BIN` / `PDFTOPPM_BIN` | Пути к внешним OCR-утилитам. | OCR-функции extractor.rs |
| `DOC_SEARCHER_DIAG_PDF` | (Удалено) путь для диагностических тестов `diag_*` — тесты вычищены в Task G cleanup. | — |

### 2.4 Команды сборки/проверки

```powershell
# Windows — GUI (основная конфигурация разработки)
cd src
cargo run --features windows-desktop
cargo test --bins --features windows-desktop
cargo clippy --all-targets --features windows-desktop -- -D warnings

# Ядро (без UI)
cargo test --lib --no-default-features
cargo clippy --lib --tests --no-default-features -- -D warnings

# Форматирование (ВАЖНО: запускать из src/, иначе «lib.rs does not exist»)
cargo fmt --check
cargo fmt
```

### 2.5 settings.conf

Файл `settings.conf` рядом с бинарём хранит последний выбранный путь сканирования
(`persist_scan_path` в `app_win.rs`). При запуске GUI читается
`DOCUMENT_SEARCHER_INITIAL_DIR`, затем `settings.conf`, затем дефолт.

---

## 3. Хронология диалога (промпты дословно + что сделано)

Промпты пользователя приведены **verbatim**. Ответы ассистента — краткая техническая
запись (файлы/функции/проверки). Переключения модели (`/model …`) включены в
хронологию без анализа.

> Нумерация `USER#N` соответствует порядку в JSONL-транскрипте (см. приложение 12.3).
> Дублирующие повторы одного промпта (из-за interrupt/переключений модели) сгруппированы.

### Task A — Аудит документации
> **USER#6/#21:** «Исследуй данный проект на противоречия в документах. Основные
> документы: @./README.md и @./docs/document-searcher-documentation.md . Там могут
> быть неверные ссылки на документы или факты по проекту, которые нужно fact-check
> и исправить. Сделай это, пожалуйста.»
> (AskUserQuestion → «Минимальные исправления (Рекомендуется)»)

**Сделано:** починены 3 битых ссылки (`UNIFIED-ARCHITECTURE-SPEC.md`,
`ALT-LINUX-RUN.md`, `specs/requirements-spec.md`), обновлена диаграмма структуры;
из `rust-toolchain.toml` удалено неэффективное `version = "1.78.0"` (rustup его
игнорирует, использует `channel = "stable"`). Минимальные правки, без переписывания.

### Task B — Принятие роли + верификация качества
> **USER#222/#224:** Дословно вставлена полная роль «Principal Rust Software
> Architect & Systems Engineer» (type-driven design, dual-runtime, no blocking in
> async, cancellation/generation guards, graceful shutdown, feature-free core,
> additive Cargo features, build.rs/MSRV, thiserror/anyhow, batch resilience,
> no runtime panics on user data/IO/network, UTF-8 char-boundary safety, honest
> verification — код должен проходить `cargo fmt --check`,
> `cargo clippy -- -D warnings`, `cargo test`).

**Сделано:** принята роль. Исправлены clippy-предупреждения:
`manual_is_multiple_of` (→ `is_multiple_of(2)` idiom, Rust 1.99+) в `extractor.rs:274,474`
и `worker.rs:248`; `collapsible_match`/`collapsible_if` свёрнуты (`extractor.rs:587`).
`cargo fmt`/`clippy`/`test` — зелёные (на тот момент).

### Task C — Кросс-платформенный `app_win.rs`
> **USER#325/#328/#334:** «Теперь, являясь тем же экспертом, сделай следующее:
> прочитай файлы @..\..\.claude\plans\adapt-to-all-os-implementation-plan.md и
> @..\..\.claude\responses\adaptation-to-os-plan-response.md . implementation plan -
> план реализации кода, который будет адаптирован по разные ОС, включая Windows,
> Alt Linux, Ubuntu. plan response - ответ от ии с почти готовым кодом. Нужно
> дозаполнить файл src/src/app_win.rs внутри этого ответа. Напиши код и протестируй
> его здесь, на Windows. Все необходимые команды для запуска "cargo ..." ты знаешь.
> Приступай.»

**Сделано:** Linux-заглушка `mod win32` заменена полноценным адаптером:
`pick_folder` (zenity→kdialog→yad с `--filename=<dir>/`, `Stdio::null()`),
`open_with_default_app` (xdg-open→gio open), `silence_stdout_spam` (`dup2`→dev_null
под unix), `find_app_browser`/`spawn_desktop_window` кросс-платформенные
(поиск chromium/google-chrome/yandex-browser/brave/microsoft-edge в
`/usr/bin`, `/usr/local/bin`, `/snap/bin`; fallback `firefox --new-window`).
Добавлен эндпоинт `POST /api/set-folder` + UI поле ввода пути (кнопка `Go`).
> **USER#500/#501:** (interrupt) «У тебя уже есть план. Он выше: [вставлен мой
> собственный план]» — записан план, вызван ExitPlanMode.
> **USER#618/#621/#638/#640:** «продолжи реализацию плана» (после переключений модели).

### Task D — Обновление README
> **USER#733/#735:** «Да, теперь измени README.md в соответствии с текущими
> изменениями в коде, чтобы там были актуальные инструкции по запуску, развёртыванию
> и структуры папок.»

**Сделано:** крупный rewrite README — кросс-платформенный GUI, поле ввода пути,
`--features windows-desktop` для Linux GUI (критично: `default=[]`, иначе только CLI),
таблица фич, таблица env-vars, реструктурированные Linux-зависимости по назначению.

### Task E — Робастное декодирование текста
> **USER#844:** (англ.) «I continue testing the application. The text sometimes
> extracts properly and searches well, but on some documents that I personally know
> as valid .md or .pdf files aren't opened and neither searched through. How modify
> this behaviour and make it read more the files in a more stable way. Here are the
> logs: [WARN Skipping file … error=Text file is unreadable or is not UTF-8 …
> stream did not contain valid UTF-8]»

**Сделано:** root cause — `fs::read_to_string` падал на CP1251-файлах (русский
Windows). Реализован 4-слойный lossless-декодер `decode_text_bytes` (`extractor.rs:307`):
(1) BOM fast-path UTF-8/UTF-16LE/UTF-16BE, (2) strict UTF-8 fast-path + NUL-ratio guard,
(3) `chardetng` detection + `encoding_rs` lossy decode, (4) UTF-16 rescue (если на
выходе много NUL — retry UTF-16LE/BE, оставить вариант с меньшим числом NUL).
`extract_plain_text` переписан на `fs::read` + `decode_text_bytes` (`extractor.rs:282`).
В `Cargo.toml` добавлены `encoding_rs = "0.8"`, `chardetng = "0.1"`.
Тест: `extracts_cp1251_plain_text_from_disk` — pass.
> **USER#863/#872/#908:** продолжения сессии после compaction (summary-блоки).

### Task F — Регрессия поиска PDF (первая попытка)
> **USER#1150:** «Я говорю: теперь .pdf файлы вообще почти не ищутся. Я знаю, какой
> там текст, и знаю, что он там есть. Когда я открываю document-searcher . , он не
> находит по ключевыми словами нужный текст. То есть, даже при том , что он там есть,
> и это приложение как бы поддерживает этот формат. Разберись с проблемой.»

**Сделано (Task F):** перебалансирован `extract_pdf_text` (`extractor.rs:922`):
теперь `pdf-extract` **первым** (лучше границы слов), `lopdf` как fallback,
затем OCR. catch_unwind вокруг pdf-extract.
> **USER#1274–1312:** «продолжи работу. Ты заметил эту ошибку: [вставлен вывод
> диагностического теста `diag_pdf_pipeline` для The-Rust-PL.pdf — 626 страниц,
> lopdf load OK, постраничные builtin_alpha/custom_alpha counts]»

**Сделано (диагностика):** добавлены временные `#[ignore]`-тесты `diag_pdf_pipeline`
и `diag_pdf_search_word_breaks` для исследования word-break проблем. **Позже удалены**
(Task G cleanup, см. ниже) — они были debug-only и остались висящими вне `mod tests`,
вызывая `unexpected closing delimiter }`.

### Task G — OCR + номера страниц + стриминг + переход на страницу
> **USER#1650/#1653:** «Теперь появилась другая проблема: поиск по pdf документу
> без явного текста проваливается. Довольно часто .pdf файлы попадают не со сплошным
> текстом, который можно копировать и вставлять, а с картинками, на которых
> расположен сам текст. Я хочу, чтобы наше приложение оцифровывало текст с картинок
> и также выполняла поиск по ним. Также я хочу добавить страницы для документов
> .docx и .pdf в результатах поиска, чтобы было удобней искать. Проблема с этим
> заключатеся в том, как определить номер страницы, чтобы чётко перейти по ней.
> Каждый pdf-viewer показывает текующую страницу, которую он просматривает. Есть ли
> программный способ узнать сколько страниц всего и где каждая из них. Когда
> происходит оцифровка .pdf и поиск по нему, в результате должна также отображаться
> страница справа от текста(Стр. N - по правую сторону от текста, который сам
> слева). Также я хочу, чтобы по ходу поиска отображались некоторые документы, где
> было найдено вхождение текста в них. Т.е. не чтобы в конце сразу все файлы
> отображались в results, а последовательно, по ходу выполнения поиска программно.
> Т.е. так: поиск нашёл вхождение текста в файле - добавил в results. Нашёл второе
> вхождение в тот же файл - добавил также и его. Нашёл в другом файле - добавил
> файл в results. Продумай, как реализовать эти требования, составь план и
> приступай к выполнению. Пока работай только над программным кодом в качестве
> Rust engineering expert. Также было бы удобно, если бы после того, как показана
> страница, где найдено данное вхождение (вписанное в поиск), можно было бы
> открывать ссылку и там pdf viewer сразу перелистывал на нужную страницу. Также
> желательно сделать с .docx файлами. Всё это нужно оставить кросс-платформенным,
> если возможно. Внеси эти изменения в приложение по поиску документов.»

**Сделано (Task G):**
- **Извлечение с страницами:** `ExtractedPage { number: u32, text: String }`
  (`state.rs:13`); поле `page: Option<u32>` в `SearchOccurrence` (`state.rs:28`).
  Новый кешированный API `extract_text_with_pages` (`extractor.rs:112`) + `PAGES_CACHE`
  (`extractor.rs:39`, mirrors `TEXT_CACHE`). `extract_pdf_text_with_pages`
  (`extractor.rs:175`) — lopdf постранично, pdf-extract fallback (плоско, page 1),
  затем OCR-fallback `extract_scanned_pdf_text_with_pages` (`extractor.rs:1220`).
- **OCR:** `extract_scanned_pdf_text_with_pages` парсит номер страницы из
  `page-NN.png` (через `rsplit('-')`), сортирует по номеру; WinRT-fallback на
  Windows (`extract_scanned_pdf_via_windows_winrt`, `extractor.rs:976`).
- **Поиск по страницам:** `search_files` (`search.rs:30`) итерирует страницы,
  ставит `occ.page = Some(page.number)` (`search.rs:60`).
- **Стриминг:** `WorkerEvent::SearchResultFound { result, completed, total }`
  (`actions.rs:30`); воркер шлёт событие на каждый файл по завершении JoinSet
  (`worker.rs:250–268`); финальный `SearchCompleted` шлёт `results: Vec::new()`
  (`worker.rs:293–296`, все результаты уже streamed).
- **UI приём:** `search_result_found` (`app_win.rs:663`) — de-dup по relative_path
  (мерджит occurrences); `search_completed` (`app_win.rs:690`) применяет батч
  только если non-empty.
- **Переход на страницу (первая версия):** `open_file_at_page` строил
  `file:///path#page=N` через `build_pdf_page_url`; `/api/open-file` парсит
  `path|page`; JS `openFileAtPage` (`app_win.rs:2062`); CSS `.page-number`.
- **JSON:** `serialize_state_json` (`app_win.rs:793`) добавляет `,"page":N` в
  occurrence-JSON (`app_win.rs:870`).
> **USER#1639:** «Я тестирую на Windows, по идее, работает неплохо. Будет ли
> работать тот же самый код для Alt Linux / Ubuntu?» — отвечено архитектурно
> (zenity/xdg-open/chromium ветки), но Linux-ветка реально не компилировалась.
> **USER#1885/#1888:** «продолжи».
> **USER#1920–1926:** (interrupt + paste прогресса) — продолжена финальная
> проверка/тестирование Task G.

**Cleanup Task G (Phase 1 этой сессии):** удалены временные диагностические тесты
`diag_pdf_pipeline` и `diag_pdf_search_word_breaks` из `extractor.rs` (они
оставались висящими вне `mod tests`, вызывая `unexpected closing delimiter }`).
Файл теперь заканчивается чисто на строке 1457. После этого: 24 lib + 5 bin теста pass.

### Task H — Регрессия PDF (вторая) + меню расширений + пробелы + Yandex Browser
> **USER#2401/#2404/#2437:** «Я не знаю, что мы сделали с ядром, но раньше оно хотя
> бы выдавало правильно текст на тех .pdf документах, внутри которых был явный
> текст. Теперь, после того, как мы попытались внедрить последовательное появление
> документов и внедрить OCR, он перестал вообще искать файлы в некоторых папках.
> Пока идём хорошо: мне нравится панель с переходом на другую папку, страницы
> отображаются корректно для .pdf файлов. Но пока происходит ошибка, когда
> переходишь по ним. У меня .pdf файлы открываются в Yandex Browser в pdf viewer,
> встроенный там. Вот, как выглядит интерфейс: [Image #14]. Вот ещё несколько
> нюансов: 1. давай в интерфейсе на панеле рядом с кнопками перехода на другую
> папку сделаем меню с выбором расширений, которые исчет система. 2. когда
> выдаётся результат текстовый вхождений подстроки в файле, то там появляются
> лишние пробелы. Иногда они не мешают, а иногда сбивают поиск по документу при
> копировании. Исправь эти недочёты и учти текущее положение дел. Всё хорошо, но
> нужно продолжать разрабатыввать систему в соответствии с этими замечаниями.»

**Сделано (Task H):**
- **(a) PDF-регрессия:** гипотеза — `extract_pdf_text_with_pages` делает
  `Document::load(...)?` **первым** (lopdf), и при ошибке lopdf **не пробует
  pdf-extract вообще** → файл пропускается целиком (тогда как старый
  `extract_pdf_text` пробовал pdf-extract первым). Частичная правка: убран фильтр
  `if !page_text.trim().is_empty()` — теперь все страницы пушатся всегда
  (`extractor.rs:193–199`), сохраняя нумерацию. **Глубже проблема (lopdf-first +
  hard bail) НЕ исправлена и НЕ проверена на реальных PDF.**
- **(b) Yandex Browser page-link:** `open_file_at_page` (`app_win.rs:499`)
  переписан — **игнорирует** `page` (`let _ = page;`), открывает файл как есть.
  `build_pdf_page_url` удалена (но **её doc-comment остался orphaned** — см.
  раздел 9, причина падения clippy). Номер страницы только отображается в UI.
- **(c) Меню расширений:** CSS `.extensions-row`/`.extension-chip`/`.active`/
  `.inactive` (`app_win.rs:1639–1667`); HTML `<div id="extensionsList">`
  (`app_win.rs:1933`); JS `ALL_EXTENSIONS`/`activeExtensions` Set (дефолт
  `docx,pdf`)/`renderExtensionChips`/`notifyExtensionsChanged`→`postApi("/api/set-extensions")`
  (`app_win.rs:2011–2040`); эндпоинт `POST /api/set-extensions` (`app_win.rs:1309`)
  парсит CSV, `std::env::set_var("DOCUMENT_SEARCHER_EXTENSIONS", …)` (`app_win.rs:1328`),
  пересканирует текущую папку. В `scanner.rs` добавлены `DEFAULT_EXTENSIONS` и
  `ALL_SUPPORTED_EXTENSIONS` консты (`scanner.rs:10,13`), чтение env-var
  (`scanner.rs:52`).
- **(d) Лишние пробелы:** добавлена `normalize_whitespace` (`extractor.rs:862`) —
  схлопывает множественные пробелы/табы → один пробел, множественные newlines →
  один, тримит. **Применена только в 2 местах:** `extract_single_page_text`
  (builtin lopdf, `extractor.rs:239`) и `extract_page_custom_text` (return value).
  **НЕ применена** к pdf-extract fallback, OCR, DOCX, plain-text — проблема
  закрыта **частично**.

### Task I — Handoff-документ (текущая задача)
> **USER#2710:** «Вынеси весь диалог в отдельный файл, чтобы можно было продолжить
> разрабатывать проект с чистой сессии без контекста. Файл должен учитывать каждую
> деталь - реализованную по факту и предложенную к разработке. Можешь также завести
> таблицу: промпт - реализованные функции. Можешь дополнить эту таблицу другими
> колонками или сделать ещё одну другую. Главное, чтобы вынести всё содержимое
> диалога.»
> (AskUserQuestion → формат «Handoff + дословные промпты»; расположение
> `docs/DEVELOPMENT_HANDOFF.md`)
> **USER#2796/#2798:** «давай» (одобрение плана)

**Сделано:** этот файл. Код не трогался (по требованию пользователя — только документ).

---

## 4. Таблица «Промпт → реализованные функции»

Статусы: ✅ проверено тестами/вживую · ⚠️ компилируется, не проверено вживую · ❌ не реализовано.

| № | Промпт (краткая суть) | Что реализовано | Файлы / функции | Статус |
|---|---|---|---|---|
| A | Аудит документации на противоречия | Починены 3 битых ссылки, обновлена диаграмма; убрано неэффективное `version` из rust-toolchain | `README.md`, `docs/…`, `src/rust-toolchain.toml` | ✅ |
| B | Принять роль Rust-архитектора, проверить качество | `is_multiple_of` idiom, свёрнутые match/if; fmt/clippy/test зелёные (на тот момент) | `extractor.rs:274,474,587`, `worker.rs:248` | ✅ (устарело — см. 0) |
| C | Дозаполнить `app_win.rs` кросс-платформенным кодом (Win/Alt Linux/Ubuntu), протестировать на Windows | Linux-адаптер (zenity/kdialog/yad, xdg-open/gio, chromium/firefox), эндпоинт `/api/set-folder`, UI поле пути | `app_win.rs` (`mod win32`, `find_app_browser`, `spawn_desktop_window`, `/api/set-folder`) | ✅ Win · ⚠️ Linux не компилировался |
| D | Обновить README под текущий код | Rewrite: фичи, env-vars, кросс-платформенный GUI, `--features windows-desktop` для Linux | `README.md` | ✅ |
| E | Файлы .md/.pdf не открываются/не ищутся; сделать чтение стабильнее (логи: «not UTF-8») | 4-слойный lossless-декодер текста (BOM/UTF-8/chardetng/UTF-16 rescue); `encoding_rs`+`chardetng` в deps | `extractor.rs:282,307`; `Cargo.toml:49,50` | ✅ тест · ⚠️ не на всех кодировках вживую |
| F | PDF почти не ищутся, хотя текст есть | Перебалансировка `extract_pdf_text`: pdf-extract первым, lopdf fallback, OCR; catch_unwind | `extractor.rs:922` | ⚠️ компилируется · ❌ не проверено вживую |
| F-diag | «продолжи, ты заметил эту ошибку: [diag-вывод]» | Временные диагностические `#[ignore]`-тесты `diag_pdf_pipeline`/`diag_pdf_search_word_breaks` | (были в extractor.rs) | ✅ удалены в cleanup Task G |
| G1 | OCR сканов PDF: оцифровать текст с картинок и искать | `extract_scanned_pdf_text_with_pages` (pdftoppm→tesseract, page-NN.png parsing), WinRT-fallback на Windows | `extractor.rs:976,1220` | ⚠️ компилируется · ❌ не проверен e2e на реальном скане |
| G2 | Номера страниц .docx/.pdf в результатах, «Стр. N» справа | `ExtractedPage`, `SearchOccurrence.page`, `extract_text_with_pages`, постраничный поиск, JSON `page`, UI «Стр. N» | `state.rs:13,28`; `extractor.rs:112,175`; `search.rs:30,60`; `app_win.rs:870` | ⚠️ PDF ✅ · ❌ DOCX фиктивно «Стр. 1» |
| G3 | Стриминг результатов по ходу поиска | `WorkerEvent::SearchResultFound`, воркер стримит per-file, UI de-dup+merge | `actions.rs:30`; `worker.rs:250`; `app_win.rs:663,690` | ⚠️ компилируется · ❌ не проверен e2e в живом UI |
| G4 | Клик по странице → PDF viewer на нужной странице; кросс-платформенно | (Task G) `open_file_at_page` + `build_pdf_page_url` + `/api/open-file` `path|page`; **(Task H) упрощено: page игнорируется** из-за Yandex Browser | `app_win.rs:499,1208,2062` | ❌ no-op (см. 9) |
| G5 | «Также желательно с .docx» — страницы DOCX | DOCX возвращает единственную страницу `number=1` (без рендеринга страниц нет) | `extractor.rs:405`; `extract_text_with_pages_uncached:166` | ❌ архитектурно невозможно без рендера |
| H1 | Регрессия: «перестал искать файлы в некоторых папках» | Убран фильтр пустых страниц; **глубже (lopdf-first + hard bail) НЕ исправлено** | `extractor.rs:193` | ❌ гипотеза не подтверждена/не исправлена |
| H2 | Ошибка перехода по странице в Yandex Browser | `open_file_at_page` игнорирует page; `build_pdf_page_url` удалена (doc-comment остался) | `app_win.rs:499–513` | ⚠️ страница только отображается |
| H3 | Меню расширений рядом с кнопками папки | Chips UI (CSS/HTML/JS), эндпоинт `/api/set-extensions`, env-var, rescan, `ALL_SUPPORTED_EXTENSIONS` | `app_win.rs:1639,1933,2011,1309`; `scanner.rs:10,13,52` | ✅ компилируется · ⚠️ не проверено вживую |
| H4 | Лишние пробелы в сниппетах вхождений | `normalize_whitespace` — **только 2 вызова** (lopdf builtin + custom) | `extractor.rs:862,239,800` | ❌ частично (pdf-extract/OCR/DOCX/plain без неё) |
| I | Вынести диалог в файл для чистой сессии | Этот документ | `docs/DEVELOPMENT_HANDOFF.md` | ✅ |

---

## 5. Реестр функций (только реально существующие символы)

### 5.1 `model/state.rs`

| Функция/тип | Назначение | Статус |
|---|---|---|
| `FileEntry { relative_path, absolute_path }` | Запись проиндексированного файла | ✅ |
| `ExtractedPage { number: u32, text: String }` | Страница с текстом (1-indexed) | ✅ добавлено Task G |
| `SearchOccurrence { before, matched, after, snippet, page: Option<u32> }` | Одно вхождение с контекстом | ✅ `page` добавлен Task G |
| `SearchMatch { file_relative_path, file_absolute_path, contexts, occurrences }` | Все вхождения в одном файле | ✅ |
| `AppState` + методы (`new`, `set_scan_path`, `reset_search`, `get_visible_results`, `has_more_results`, `load_more`) | UI-состояние, пагинация по 100 | ✅ |

### 5.2 `model/scanner.rs`

| Функция/тип | Назначение | Статус |
|---|---|---|
| `DEFAULT_EXTENSIONS = ["docx","pdf"]` | Дефолтный список расширений | ✅ добавлено Task H |
| `ALL_SUPPORTED_EXTENSIONS = [docx,pdf,txt,md,rs,toml,json,xml,html,css,js,ts]` | Полный список для UI | ✅ добавлено Task H |
| `MAX_FILE_SIZE = 50 MiB` | Лимит размера файла | ✅ |
| `scan_directory`, `scan_directory_with_report`, `scan_directory_with_extensions` | walkdir-обход с фильтром | ✅ `scan_directory_with_report` читает env-var `DOCUMENT_SEARCHER_EXTENSIONS` (scanner.rs:52) |
| `is_allowed_extension`, `is_supported_extension`, `is_visible_entry` | Фильтры расширений/видимости | ✅ |

### 5.3 `model/extractor.rs`

| Функция | Назначение | Статус |
|---|---|---|
| `normalize_platform_path(path)` | Убирает `\\?\` / `\\?\UNC\` префиксы canonicalize | ✅ |
| `extract_text(path)` | Кешированное извлечение текста (TEXT_CACHE) | ✅ |
| `extract_text_with_pages(path)` | Кешированное извлечение со страницами (PAGES_CACHE) | ✅ Task G (extractor.rs:112) |
| `extract_text_with_pages_uncached` | Диспетчер: PDF → `extract_pdf_text_with_pages`; иначе single page | ✅ (extractor.rs:156) |
| `extract_pdf_text_with_pages(path)` | lopdf постранично → pdf-extract fallback → OCR fallback | ⚠️ (extractor.rs:175) — гипотеза regression, см. 9 |
| `extract_pdf_text(path)` | Старый flat-путь: pdf-extract первым → lopdf → OCR (используется CLI/legacy) | ✅ (extractor.rs:922) |
| `extract_single_page_text(doc, page_num, page_id)` | builtin lopdf + custom, выбирает больше алфавитных; нормализует whitespace | ✅ (extractor.rs:234) |
| `extract_page_custom_text(doc, page_id)` | Кастомный декодер операторов PDF (BT/ET/Tj/TJ), глифы uni04XX/afii/ToUnicode/CP1251; `normalize_whitespace` на return | ✅ (extractor.rs:800) |
| `extract_pdf_with_lopdf(path)` | Flat lopdf-извлечение (fallback для legacy) | ✅ (extractor.rs:897) |
| `normalize_whitespace(text)` | Схлопывает множественные пробелы/newlines, тримит | ✅ (extractor.rs:862) — **только 2 вызова** |
| `count_alphabetic(text)` | Счётчик алфавитных символов (эвристика «есть ли текст») | ✅ (extractor.rs:893) |
| `extract_plain_text(path)` | `fs::read` + `decode_text_bytes` | ✅ Task E (extractor.rs:282) |
| `decode_text_bytes(bytes)` | 4-слойный lossless-декодер (BOM/UTF-8/chardetng/UTF-16 rescue) | ✅ Task E (extractor.rs:307) |
| `extract_docx_text(path)` | zip→`word/document.xml`→quick-xml извлечение текста | ✅ (extractor.rs:405) — **без пагинации** |
| `extract_scanned_pdf_text_with_pages(path)` | OCR постранично (pdftoppm page-NN.png → tesseract) | ⚠️ cfg(any(ocr,windows-ocr)) (extractor.rs:1220) + stub без фич (extractor.rs:1336) |
| `extract_scanned_pdf_text(path)` | OCR flat (делегирует pages-версию, объединяет) | ⚠️ (extractor.rs:1199/1330) |
| `extract_image_text(path)` | OCR одного изображения | ⚠️ (extractor.rs:1175/1194) |
| `extract_scanned_pdf_via_windows_winrt(path)` | WinRT OCR fallback на Windows | ⚠️ cfg windows (extractor.rs:976) |
| `decode_glyph_name(name)` | Декодер имён глифов (uni041B→Л, afii10017→А, .cap) | ✅ (тест `decodes_cyrillic_uni_and_afii_glyph_names`) |
| `TEXT_CACHE` / `PAGES_CACHE` | `OnceLock<RwLock<HashMap<CacheKey, …>>>`, CacheKey=(path,len,mtime) | ✅ (extractor.rs:38,39) |
| `cache_store()` / `pages_cache_store()` | Доступ к кэшам; лимит 4096 записей, очистка при переполнении | ✅ |

### 5.4 `model/search.rs`

| Функция | Назначение | Статус |
|---|---|---|
| `search_files(files, query)` | Извлечение со страницами + поиск per-page + `occ.page=Some` | ✅ (search.rs:30) |
| `find_occurrences_with_context(text, folded_query)` | Unicode-caseless поиск, контекст ±20, UTF-8 char-boundary safety | ✅ (search.rs:83) |
| `FoldedText` | Маппинг folded-индексов → source spans (char_indices) | ✅ |

### 5.5 `model/actions.rs`

| Тип | Назначение | Статус |
|---|---|---|
| `WorkerCommand { Scan, QueryChanged, Shutdown }` | Команды воркеру | ✅ |
| `WorkerEvent` | `ScanStarted`/`ScanProgress`/`ScanCompleted`/`ScanError`/`SearchStarted`/`SearchResultFound`/`SearchProgress`/`SearchCompleted`/`SearchCancelled`/`SearchError` | ✅ `SearchResultFound` добавлен Task G (actions.rs:30) |
| `AppMsg` | Сообщения UI | ✅ |

### 5.6 `model/worker.rs`

| Функция/метод | Назначение | Статус |
|---|---|---|
| `Worker` + `WorkerHandle` | tokio-воркер, JoinSet, debounce 300ms, CancellationToken, AtomicU64 generation | ✅ |
| `start_search` loop | Стримит `SearchResultFound` per-file; финал `SearchCompleted { results: Vec::new() }` | ✅ (worker.rs:250–298) |
| `send_search_event` | Отправка события с проверкой cancel/generation | ✅ |

### 5.7 `app_win.rs` (UI, HTTP, JS/CSS)

| Функция/элемент | Назначение | Статус |
|---|---|---|
| `open_file_at_page(path, page)` | Открывает файл в программе по умолчанию; **игнорирует page** | ⚠️ no-op для page (app_win.rs:499) |
| `normalize_platform_path` (re-export) | Убирает `\\?\` префиксы | ✅ |
| `serialize_state_json(ui)` | Ручная JSON-сериализация состояния; `page` поле в occurrence | ✅ (app_win.rs:793,870) |
| `search_result_found(result)` | Append с de-dup по relative_path, мердж occurrences | ✅ (app_win.rs:663) |
| `search_completed(query, results)` | Применяет батч только если non-empty | ✅ (app_win.rs:690) |
| `find_app_browser` / `spawn_desktop_window` | Кросс-платформенный запуск окна браузера (--app mode) | ✅ Win · ⚠️ Linux не компилировался |
| `mod win32` | Win32 (SHBrowseForFolderW, ShellExecuteW) + Linux (zenity/kdialog/yad, xdg-open/gio) | ✅ Win · ⚠️ Linux |
| `Controller` / `WinUiState` / `HttpWorkerPool` / `RuntimeBridge` | Синхронизация UI↔воркер, 4 HTTP-потока, dedicated tokio thread | ✅ |
| Эндпоинты | См. раздел 6 | ✅ |
| JS `ALL_EXTENSIONS`/`activeExtensions`/`renderExtensionChips`/`notifyExtensionsChanged` | UI меню расширений | ✅ Task H (app_win.rs:2011–2040) |
| JS `openFileAtPage(absPath, page)` | POST `/api/open-file` с `path|page` | ✅ (app_win.rs:2062) |
| CSS `.extensions-row`/`.extension-chip`/`.active`/`.inactive`/`.path-input-row`/`.page-number` | Стили UI | ✅ |

---

## 6. API endpoints (`app_win.rs` `handle_client`)

Все POST требуют заголовок `X-Document-Searcher-Token: <SESSION_TOKEN>` (CSRF).
`/api/query` дополнительно шлёт `X-Query-Seq: <N>` (anti-race).

| Метод | Путь | Тело | Назначение | Auth |
|---|---|---|---|---|
| GET | `/` | — | Отдаёт HTML UI | — |
| GET | `/api/state` | — | JSON состояния (files, results, page в occurrences) | — |
| POST | `/api/query` | query string | Запуск поиска (debounce, generation guard) | token + seq |
| POST | `/api/load-more` | — | Пагинация +100 результатов | token |
| POST | `/api/open-file` | `path` или `path\|page` | Открыть файл (page **игнорируется** сейчас) | token + whitelist (`is_indexed_file`) |
| POST | `/api/choose-folder` | — | Системный диалог выбора папки | token |
| POST | `/api/set-folder` | path string | Ручной ввод пути → canonicalize → rescan | token |
| POST | `/api/set-extensions` | `docx,pdf,txt` | `std::env::set_var(…)` + rescan текущей папки | token |
| POST | `/api/shutdown` | — | Запрос завершения | token |

Строки: `/api/state`:1181, `/api/query`:1196, `/api/load-more`:1202, `/api/open-file`:1208,
`/api/choose-folder`:1243, `/api/set-folder`:1283, `/api/set-extensions`:1309,
`/api/shutdown`:1340.

---

## 7. Модель данных и поток данных

### 7.1 Типы (`state.rs`)

```text
FileEntry { relative_path: String, absolute_path: PathBuf }
ExtractedPage { number: u32, text: String }                  // Task G
SearchOccurrence { before, matched, after, snippet: String, page: Option<u32> }  // page — Task G
SearchMatch { file_relative_path, file_absolute_path, contexts: Vec<String>, occurrences: Vec<SearchOccurrence> }
AppState { settings, current_path, found_files, search_query, search_results, results_offset, results_limit:100, status, error, skipped_* }
```

### 7.2 Воркер-протокол (`actions.rs`)

```text
WorkerCommand { Scan{root}, QueryChanged{query}, Shutdown }
WorkerEvent {
    ScanStarted, ScanProgress{completed}, ScanCompleted{files, oversized, unreadable}, ScanError,
    SearchStarted{query},
    SearchResultFound{result: SearchMatch, completed, total},   // Task G — стриминг per-file
    SearchProgress{completed, total},
    SearchCompleted{query, results: Vec<SearchMatch>},          // results может быть пустым (всё уже streamed)
    SearchCancelled{query}, SearchError{query, error}
}
```

### 7.3 Пайплайн

```text
[scan_directory_with_report]  ── env DOCUMENT_SEARCHER_EXTENSIONS ──►  FileEntry[]
        │
        ▼
[extract_text_with_pages]  ── PAGES_CACHE (path,len,mtime) ──►
        │   PDF: extract_pdf_text_with_pages
        │       ├─ lopdf Document::load → extract_single_page_text per page (builtin + custom)
        │       │   └─ normalize_whitespace applied (builtin + custom return)
        │       ├─ if total_alpha<32 → pdf-extract flat (page 1)  ← БЕЗ normalize_whitespace
        │       └─ else → extract_scanned_pdf_text_with_pages (OCR per page-NN.png)
        │   non-PDF: extract_text → single page (number=1)
        │       ├─ plain: decode_text_bytes (4-layer)
        │       ├─ docx: extract_docx_text (zip+xml, БЕЗ пагинации)
        │       └─ image: OCR
        ▼
Vec<ExtractedPage>
        │
        ▼
[search_files]  per-page find_occurrences_with_context → occ.page = Some(page.number)
        │
        ▼
[worker] JoinSet per-file → WorkerEvent::SearchResultFound (streamed) → финал SearchCompleted{results:Vec::new()}
        │
        ▼
[app_win] search_result_found: de-dup by rel_path + merge occurrences → AppState.search_results
        │   polling UI каждые 120мс → GET /api/state → serialize_state_json (page в occurrence JSON)
        ▼
UI: карточки результатов, «Стр. N» справа, клик по странице → openFileAtPage (page сейчас no-op)
```

---

## 8. Статус верификации (честно)

### Проверено (на Windows, 2026-11-09)

- ✅ `cargo test --lib --no-default-features` — **24 passed, 0 failed**.
- ✅ `cargo test --bins --features windows-desktop` — **5 passed, 0 failed**.
- ✅ `cargo clippy --lib --tests --no-default-features -- -D warnings` — **чисто**.

### Не проверено

- ❌ **`cargo fmt --check`** — ПАДАЕТ (2 файла, см. 0 и 9).
- ❌ **`cargo clippy --all-targets --features windows-desktop -- -D warnings`** — ПАДАЕТ
  (orphaned doc-comment `app_win.rs:515–517`, см. 9).
- ⚠️ OCR на реальном отсканированном PDF — ни разу не запускался e2e.
- ⚠️ Стриминг результатов в живом UI — не проверен end-to-end (логика есть, UI-поведение не тестировалось).
- ⚠️ Переход на страницу — сейчас no-op (`let _ = page;`); номер только отображается.
- ⚠️ Linux-ветка `app_win.rs` (`zenity`/`xdg-open`/`chromium`) — **ни разу не компилировалась** в этой сессии (только Windows).
- ⚠️ `app.rs` (GTK/relm4, фича `desktop`) — не компилировалась.
- ⚠️ Поведение `std::env::set_var` в многопоточном процессе (race с чтением из воркера) — не тестировалось.

### Тесты, которых нет

- Нет тестов на `extract_text_with_pages` / `extract_pdf_text_with_pages` (страницы).
- Нет тестов на OCR-функции.
- Нет тестов на стриминг `SearchResultFound` (только `scan_then_search_returns_results` проверяет итог).
- Нет тестов на `/api/set-extensions`.

---

## 9. Известные проблемы и технический долг

| Приоритет | Проблема | Причина | Предлагаемое решение | Файлы:строки |
|---|---|---|---|---|
| **P0** | `cargo fmt --check` падает | После правок Task H `cargo fmt` не перезапускался | `cargo fmt` (из `src/`) | `extractor.rs:242`, `app_win.rs:1311` |
| **P0** | `cargo clippy --all-targets --features windows-desktop -- -D warnings` падает (`empty_line_after_doc_comment`) | При удалении `build_pdf_page_url` в Task H её doc-comment (3 строки `///`) остался висящим над `struct WinUiState` | Удалить строки 515–517 (orphaned doc-comment) | `app_win.rs:515–517` |
| **P0** | Регрессия поиска PDF: «перестал искать файлы в некоторых папках» | `extract_pdf_text_with_pages` делает `Document::load(path)?` **первым** (lopdf) и при ошибке lopdf **не пробует pdf-extract вообще** — весь файл пропускается. Старый `extract_pdf_text` пробовал pdf-extract первым. | В `extract_pdf_text_with_pages` обернуть `Document::load` в `match`: при ошибке lopdf → fallback на pdf-extract flat (как в `extract_pdf_text:922`), затем OCR. **Не проверено на реальных PDF.** | `extractor.rs:175–231` |
| **P1** | Переход на страницу — no-op | Yandex Browser (встроенный PDF viewer) не поддерживает `file://…#page=N`. `open_file_at_page` игнорирует `page`. | Варианты: (a) CLI-параметры вьюеров (`/A page=N` для Adobe, `--page=N`); (b) встроенный PDF.js-вьюер в окне приложения; (c) Chromium `--app` с URL `#page=N`. Требует решения пользователя. | `app_win.rs:499–513` |
| **P1** | «Лишние пробелы» исправлены частично | `normalize_whitespace` применена только к lopdf builtin + custom. pdf-extract fallback, OCR, DOCX, plain-text — без нормализации. | Применить `normalize_whitespace` к: pdf-extract тексту (`extractor.rs:221`), OCR-выходу, DOCX-выходу (`extract_docx_text:405`), `decode_text_bytes` выходу (`extract_plain_text:282`). | `extractor.rs:862` (только 2 вызова: `:239`, `:800`) |
| **P1** | DOCX показывает фиктивную «Стр. 1» | DOCX не имеет понятия страниц без рендеринга. Возвращается single page `number=1`. | Либо убрать отображение «Стр.» для DOCX (когда `pages.len()==1 && extension!=pdf`), либо рендеринг DOCX→PDF/страницы (тяжело). | `extractor.rs:405`, `extract_text_with_pages_uncached:166` |
| **P2** | `std::env::set_var` в многопоточном процессе | `/api/set-extensions` пишет env-var, которую воркер читает в `scanner.rs:52` — race condition (env-var не thread-safe для записи после старта потоков). | Хранить выбор расширений в `Controller`/`AppState`, передавать в scanner параметром, не через env. | `app_win.rs:1328`, `scanner.rs:52`, `main.rs:94` |
| **P2** | Дефолт чипов в JS хардкод `["docx","pdf"]` vs `--ext` | Если запуск с `--ext txt,md`, JS всё равно покажет активными docx+pdf (рассинхрон). | Сервер должен отдавать текущий активный список в `/api/state`, JS рендерит по нему. | `app_win.rs:2012` |
| **P2** | Пересканирование на каждый клик чипа | Каждый toggle чипа → `POST /api/set-extensions` → rescan всей папки. | Debounce на toggle, или rescan только при явном Apply. | `app_win.rs:2037` |
| **P2** | `app_win.rs:515–517` orphaned doc-comment | (та же, что P0 clippy) — отдельный симптом удаления `build_pdf_page_url` | Удалить doc-comment | `app_win.rs:515–517` |
| **P3** | Нет тестов на страницы/OCR/стриминг/set-extensions | — | Добавить unit-тесты на `extract_text_with_pages` (фикстуры PDF), интеграционный тест на streaming, тест на `/api/set-extensions`. | — |
| **P3** | Временные файлы `_user_msgs.txt`/`_user_msgs_clean.txt` в корне | Артефакты извлечения промптов для этого документа | Удалить после ревью документа | корень репо |

---

## 10. Backlog (предложено к разработке, не реализовано)

1. **Открытие PDF на странице (варианты):**
   - CLI-параметры вьюеров: Adobe `/A page=N`, Foxit, Sumatra `-page N`.
   - Встроенный PDF.js-вьюер в окне приложения (`--app` URL с `#page=N`).
   - Определение вьюера по умолчанию и диспетчеризация.
2. **Пагинация DOCX:** рендеринг DOCX→PDF (libreoffice headless?) или разбиение по
   секциям/разрывам страниц в XML (`w:br w:type="page"`). Тяжело, требует решения.
3. **Персистентность выбранных расширений:** сохранять в `settings.conf`, не в env-var.
4. **e2e-проверка OCR:** тест-фикстура отсканированного PDF → assert текст найден.
5. **Тест-фикстуры:** набор PDF разных типов (текстовый, скан, кириллица,损坏енный).
6. **Отказ от env-var `DOCUMENT_SEARCHER_EXTENSIONS`** в пользу поля `Controller`/`AppState`.
7. **Синхронизация дефолта чипов JS с сервером** (через `/api/state`).
8. **Применение `normalize_whitespace`** ко всем путям извлечения (см. P1).
9. **Fallback lopdf→pdf-extract в `extract_pdf_text_with_pages`** (см. P0 regression).
10. **Компиляция и проверка Linux-ветки** (`app_win.rs` zenity/xdg-open/chromium) —
    ни разу не собиралась.
11. **Компиляция `app.rs` (GTK/relm4, фича `desktop`)** — не проверялась.

---

## 11. Открытые вопросы к пользователю

1. **Переход на страницу в Yandex Browser.** Yandex Browser игнорирует `#page=N`.
   Приемлем ли встроенный PDF.js-вьюер в окне приложения? Или CLI-параметры конкретного
   вьюера (тогда нужно знать, какой используется)?
2. **DOCX и страницы.** Показывать ли «Стр. 1» для DOCX (текущее поведение, фиктивно),
   или скрыть номер страницы для не-PDF форматов? Полноценная пагинация DOCX требует
   рендеринга — готов ли пользователь на эту сложность?
3. **Хранение выбора расширений.** Перенести ли в `settings.conf` (персистентно) и
   `AppState` (вместо env-var)? Это решит P2 race и рассинхрон JS-дефолта.
4. **Регрессия PDF.** Подтвердить ли гипотезу (lopdf-first + hard bail) на конкретном
   проблемном PDF? Если да — применить fallback-патч (P0).
5. **Пересканирование на клик чипа.** Устроит ли debounce/Apply-кнопка вместо rescan
   на каждый toggle?

---

## 12. Приложение

### 12.1 Полный список промптов сессии (verbatim)

> Извлечено из JSONL-транскрипта (59 пользовательских сообщений после очистки от
> system-reminders/task-notifications/local-command-блоков). Дубли (из-за interrupt/
> model-switch) помечены `×N`.

1. «Исследуй данный проект на противоречия в документах. Основные документы:
   @./README.md и @./docs/document-searcher-documentation.md . Там могут быть неверные
   ссылки на документы или факты по проекту, которые нужно fact-check и исправить.
   Сделай это, пожалуйста.» ×2 (USER#6/#21)
2. (AskUserQuestion) «Минимальные исправления (Рекомендуется)»
3. `/auto-mode-setup` + вставка роли «Principal Rust Software Architect & Systems
   Engineer» (полный текст — USER#222/#224; принципы сохранены в разделе «Роль» метаданных)
4. «Теперь, являясь тем же экспертом, сделай следующее: прочитай файлы
   @..\..\.claude\plans\adapt-to-all-os-implementation-plan.md и
   @..\..\.claude\responses\adaptation-to-os-plan-response.md . implementation plan -
   план реализации кода, который будет адаптирован по разные ОС, включая Windows,
   Alt Linux, Ubuntu. plan response - ответ от ии с почти готовым кодом. Нужно
   дозаполнить файл src/src/app_win.rs внутри этого ответа. Напиши код и протестируй
   его здесь, на Windows. Все необходимые команды для запуска "cargo ..." ты знаешь.
   Приступай.» ×2 (USER#325/#328/#334)
5. (interrupt) «продолжи работу» / «продолжи реализацию плана» ×~6 (USER#476/#478/#618/#621/#638/#640)
6. (interrupt) «У тебя уже есть план. Он выше: [вставлен план]» ×2 (USER#500/#501)
7. «Да, теперь измени README.md в соответствии с текущими изменениями в коде, чтобы
   там были актуальные инструкции по запуску, развёртыванию и структуры папок.» ×2 (USER#733/#735)
8. (англ.) «I continue testing the application. The text sometimes extracts properly
   and searches well, but on some documents that I personally know as valid .md or
   .pdf files aren't opened and neither searched through. How modify this behaviour
   and make it read more the files in a more stable way. Here are the logs: [WARN
   … stream did not contain valid UTF-8 …]» (USER#844)
9. (compaction summary-блоки ×3: USER#863/#872/#908) — автоматические
10. «Я говорю: теперь .pdf файлы вообще почти не ищутся. Я знаю, какой там текст, и
    знаю, что он там есть. Когда я открываю document-searcher . , он не находит по
    ключевыми словами нужный текст. То есть, даже при том , что он там есть, и это
    приложение как бы поддерживает этот формат. Разберись с проблемой.» (USER#1150)
11. «продолжи работу» ×~3 (USER#1200/#1201)
12. «продолжи работу. Ты заметил эту ошибку: [вставлен вывод diag_pdf_pipeline для
    The-Rust-PL.pdf: 626 pages, lopdf load OK, постраничные counts]» ×~8 (USER#1274/#1275/#1278/#1282/#1284/#1293/#1295/#1298/#1300/#1309/#1312)
13. «Я тестирую на Windows, по идее, работает неплохо. Будет ли работать тот же
    самый код для Alt Linux / Ubuntu?» (USER#1639)
14. «Теперь появилась другая проблема: поиск по pdf документу без явного текста
    проваливается. Довольно часто .pdf файлы попадают не со сплошным текстом…
    а с картинками, на которых расположен сам текст. Я хочу, чтобы наше приложение
    оцифровывало текст с картинок и также выполняла поиск по ним. Также я хочу
    добавить страницы для документов .docx и .pdf в результатах поиска…
    Стр. N - по правую сторону от текста… Также я хочу, чтобы по ходу поиска
    отображались некоторые документы… последовательно… Также было бы удобно, если
    бы… можно было бы открывать ссылку и там pdf viewer сразу перелистывал на
    нужную страницу. Также желательно сделать с .docx файлами. Всё это нужно
    оставить кросс-платформенным, если возможно.» ×2 (USER#1650/#1653)
15. «продолжи» ×2 (USER#1885/#1888)
16. (interrupt + paste прогресса Task G) ×~4 (USER#1919/#1920/#1922/#1924/#1925/#1926)
17. (compaction summary-блок ×2: USER#2281/#2296)
18. «Я не знаю, что мы сделали с ядром, но раньше оно хотя бы выдавало правильно
    текст на тех .pdf документах, внутри которых был явный текст. Теперь, после
    того, как мы попытались внедрить последовательное появление документов и
    внедрить OCR, он перестал вообще искать файлы в некоторых папках. Пока идём
    хорошо: мне нравится панель с переходом на другую папку, страницы отображаются
    корректно для .pdf файлов. Но пока происходит ошибка, когда переходишь по ним.
    У меня .pdf файлы открываются в Yandex Browser в pdf viewer, встроенный там.
    Вот, как выглядит интерфейс: [Image #14]. Вот ещё несколько нюансов: 1. давай в
    интерфейсе на панеле рядом с кнопками перехода на другую папку сделаем меню с
    выбором расширений, которые исчет система. 2. когда выдаётся результат текстовый
    вхождений подстроки в файле, то там появляются лишние пробелы… Исправь эти
    недочёты и учти текущее положение дел.» ×2 (USER#2401/#2404/#2437)
19. «продолжи работу» (USER#2435)
20. «Вынеси весь диалог в отдельный файл, чтобы можно было продолжить разрабатывать
    проект с чистой сессии без контекста. Файл должен учитывать каждую деталь -
    реализованную по факту и предложенную к разработке. Можешь также завести
    таблицу: промпт - реализованные функции. Можешь дополнить эту таблицу другими
    колонками или сделать ещё одну другую. Главное, чтобы вынести всё содержимое
    диалога.» (USER#2710)
21. (AskUserQuestion) формат: «Handoff + дословные промпты (рекомендую)»;
    расположение: «`docs/DEVELOPMENT_HANDOFF.md`»
22. «давай» ×2 (USER#2796/#2798) — одобрение плана

### 12.2 Существующая документация

- `README.md` — основной (обновлён Task D).
- `docs/document-searcher-documentation.md` — детальная документация.
- `src/BUILD_INSTRUCTIONS.md` — инструкции сборки.
- `docs/old-docs-during-the-work/` — архивные спеки/архитектурные документы (specs/,
  architecture/, postmortem*.md, implementation-plan, ALT-LINUX-RUN.md).

### 12.3 Полный транскрипт сессии

- **Путь:** `C:\Users\максим\.claude\projects\C--Users--------Documents-programming-projects-document-searcher\3e6ba484-8fb3-44bc-b6d0-770dd14bbc58.jsonl`
- **Размер:** ~7.9 МБ, ~2798 строк (на момент составления).
- **Содержит:** все промпты, ответы ассистента, tool-вызовы (Read/Edit/Bash/…),
  вывод команд, system-reminders, task-notifications.
- **Как пользоваться:** открыть в текстовом редакторе с поддержкой UTF-8; каждая
  строка — отдельный JSON-объект события. Для извлечения промптов пользователя
  фильтровать по `"role":"user"` (но учитывать, что многие user-блоки —
  system-reminders/compaction-summaries, а не реальные промпты).

---

*Документ составлен как handoff для продолжения разработки с чистой сессии.
Код проекта не модифицировался при составлении этого документа (по требованию
пользователя — только документ). Все технические утверждения сверены с текущим
состоянием кода на 2026-11-09.*
