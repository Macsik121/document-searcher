# Document Searcher — Postmortem сессии 2026-11-11

> **Назначение документа.** Это исчерпывающий postmortem сессии разработки
> `document-searcher`, перенесённый в файл, чтобы можно было продолжить работу
> **с нуля, начиная с этого документа**, без живого контекста и без
> предшествующего диалога. Документ фиксирует каждое требование пользователя
> (дословно), что было реализовано по факту, точные сигнатуры функций,
> архитектурные решения, диффы по модулям, верификационные статусы и известные
> ограничения среды.
>
> **Как пользоваться с чистой сессии.**
> 1. Прочитай раздел **0 (Метаданные + горящее состояние)** — там фактический
>    статус верификации на момент записи.
> 2. Прочитай **раздел 4 (Таблица «Требование → реализация»)** — карта что
>    сделано и с каким статусом.
> 3. Прочитай **раздел 5 (Архитектура и контракты)** — подписи функций и
>    потоки данных, чтобы понимать код без чтения файлов.
> 4. **Раздел 7 (Не реализовано / backlog)** — что осталось на следующую
>    сессию.
> 5. **Раздел 8 (Известные проблемы и среда)** — что НЕ подтверждено и почему.
>
> Предыдущий handoff: `docs/DEVELOPMENT_HANDOFF.md` (сессия 2026-11-09).
> Источник полного контекста сессии — JSONL-транскрипты, пути в разделе 9.

---

## 0. Метаданные и горящее состояние

| Поле | Значение |
|---|---|
| Дата составления | 2026-11-11 |
| Ветка git | `main` |
| Последний commit | `def1e93` — «Almost working search through .docx and .pdf files. Almost there.» |
| Рабочее дерево | 11 изменённых файлов + 5 untracked; дифф `+1161 / −296` |
| Коммитов в этой сессии | **0** — пользователь не запрашивал коммит (стоящее ограничение) |
| Платформа разработки | Windows 11 Pro 10.0.26200 (win32) |
| Модели сессии | `funpay/glm-5.2` → (переключения `kr/glm-5`, `kr/deepseek-3.2`) → `funpay/glm-5.2` (финальная запись postmortem); заявленный `antigravity/gemini-3.8-flash-high` через `ANTHROPIC_MODEL` |
| Роль (стоящая) | Principal Rust Software Architect & Systems Engineer |

**Фактическое верификационное состояние (сверено вживую 2026-11-11 из `src/`):**

- ✅ `cargo fmt --check` — **чисто** (раньше падало на `extractor.rs:242` и `app_win.rs:1311`; исправлено).
- ✅ `cargo clippy --lib --tests --no-default-features -- -D warnings` — **чисто**, без вывода.
- ✅ `cargo clippy --all-targets --features windows-desktop -- -D warnings` — **чисто**, без вывода (раньше падало на orphaned doc-comment в `app_win.rs:515–517`; остаток `build_pdf_page_url` удалён вместе с функцией).
- ✅ `cargo test --lib --no-default-features` — **27 passed**, 0 failed (было 24; добавлены 3 streaming-теста).
- ✅ `cargo test --bins --features windows-desktop` — **5 passed**, 0 failed.
- ✅ CLI smoke `search ... --ext txt`: заголовок «Searching … across N files…», затем файл + `(page 1)` + сниппет в callback, затем итог «Matches for …: N files» — streaming работает.

> Все ранние заявления о «fmt/clippy красных» из `docs/DEVELOPMENT_HANDOFF.md`
> и `docs/NEXT_SESSION_PROMPT.md` теперь **устарели** — они относились к
> промежуточному состоянию до финальных правок этой сессии.

**Что НЕ подтверждено (остаётся ограничением среды, не ошибкой исходников):**
- ❌ GTK/Linux desktop-сборка (`--features desktop`) — нет GTK4/libadwaita/pkg-config на Windows.
- ❌ OCR на реальных PDF (Tesseract/pdftoppm и WinRT path) — проверен только компиляцией и unit-тестами; не запускался на реальном смешанном PDF.
- ❌ Yandex Browser deep-link `file:///...#page=N` — поведение не проверено.
- ❌ Интеграционные тесты на синтетических PDF-фикстурах — **не созданы** (backlog).

---

## 1. Что было входным состоянием

К началу сессии рабочее дерево содержало незакоммиченные правки из предыдущей
сессии (`docs/DEVELOPMENT_HANDOFF.md` от 2026-11-09) плюс утверждённый план
`~/.claude/plans/woolly-plotting-spark.md` (копия в `last-implementation-plan.md`).
Утверждённый план содержал 6 направлений:

1. Логирование: `info` → `warn`, убрать штатные INFO.
2. Единый контракт расширений + hamburger-меню в Windows UI.
3. Надёжное извлечение PDF, включая смешанные страницы (постраничный OCR).
4. Потоковый CLI на уровне ядра (`search_files_streaming`).
5. GTK-паритет общего воркера (`SearchResultFound`).
6. Открытие PDF и `ShellExecuteW code 2` (не передавать `file:///...#page=N`).

Все 6 пунктов были реализованы в предыдущей части сессии; задача этой
(продолженной) части — **сгенерировать полный postmortem с учётом всего
сделанного**, чтобы можно было продолжить разработку с нуля.

---

## 2. Утверждённый план (источник правды)

План хранится в `~/.claude/plans/woolly-plotting-spark.md` и в рабочем дереве
как `last-implementation-plan.md`. Ключевые критические файлы, перечисленные в
плане:

- `src/src/model/{constants,scanner,extractor,search,actions,worker,state}.rs`
- `src/src/main.rs`
- `src/src/app_win.rs`
- `src/src/app.rs`
- `src/Cargo.toml` — только если нужны новые зависимости (не потребовались).

Критерий приёмки (раздел «Проверка» плана):
1. `cargo fmt --check`, `cargo clippy --lib --tests --no-default-features -- -D warnings`, `cargo test --lib --no-default-features`.
2. Windows: `cargo clippy --all-targets --features windows-desktop -- -D warnings` и `cargo test --bins --features windows-desktop`.
3. GTK: `cargo check --all-targets --features desktop` только в Linux/ALT Linux с GTK4/libadwaita/pkg-config.
4. Unit-тест streaming API.
5. PDF fixtures/integration-проверки (текстовый, скан, смешанный) — **не созданы, в backlog**.
6. Ручная проверка Windows UI (dropdown, `/api/state`, streaming, открытие PDF) — не выполнена в этой сессии.
7. `git diff --check` — выполнимо, но коммит не создавался.

---

## 3. Хронология диалога (ключевые сообщения)

1. Пользователь задаёт роль Rust-архитектора и просит доработать приложение.
2. Пользователь просит прочитать `docs/NEXT_SESSION_PROMPT.md` в Plan mode и спланировать действия.
3. Пользователь уточняет, что работа по промпту/плану уже проводилась и завершилась успешно; просит сверить с `docs/DEVELOPMENT_HANDOFF.md`; разрешает инициализировать audit-pdf агент.
4. «Продолжи работу. Попробуй ещё раз запустить агента.»
5. Пользователь передаёт улучшения дословно (источник требований):
   - «Прекратить логирование INFO, а оставить только WARN и ERROR.»
   - «сделать меню по клику с иконкой трёх полосок, на которое надо нажать, и появится выпадающее меню с расширениями текстовых файлов для поиска.»
   - «исправить извлечение текста и сделать так, чтобы все pdf файлы, включая те, которые содержат текст, и те, которые содержат картинки, на которых текст, были обработаны корректно.»
   - «сделать последовательный вывод результатов поиска на уровне ядра CLI.»
   - Приложен Windows-лог со `ShellExecuteW code 2` на `file:///...#page=127`.
6. Пользователь несколько раз пишет «продолжи работу по утверждённому плану реализации».
7. Пользователь спрашивает, как обойти ограничения и вручную установить/протестировать Yandex Browser.
8. **Финальный запрос:** «Сгенерируй postmortem по этой сессии. Включи туда все детали работы, чтобы они все были учтены и можно было продолжить разработку проекта с нуля, начиная с этого postmortem. Нужно учесть всё. Запиши в файл.»

---

## 4. Таблица «Требование → реализация»

| # | Требование (дословно) | Реализация | Файл:строки | Верификация |
|---|---|---|---|---|
| 1 | «Прекратить логирование INFO, оставить только WARN и ERROR» | Fallback `EnvFilter` изменён с `"info"` на `"warn"` в обоих UI; startup-сообщение понижено с `info!` до `debug!`; `RUST_LOG` остаётся как диагностическое переопределение. | `app.rs:720`, `app_win.rs:1427` (filter); `app_win.rs:1519` (startup) | ✅ clippy/test; ⚠️ runtime не запускался вручную |
| 2 | «меню по клику с иконкой трёх полосок… выпадающее меню с расширениями» | Inline chips заменены на `.hamburger-btn` («☰ File types»), открывающий `.extensions-dropdown` popover с checkbox-элементами. Закрытие по клику вне меню и по `Escape`. Состояние гидрируется из `data.scanExtensions` (`/api/state`). | `app_win.rs` `INDEX_HTML` (CSS ~1679, HTML ~2030, JS ниже) | ✅ компилируется; ❌ ручной запуск не выполнен |
| 3 | «исправить извлечение текста… все pdf, включая смешанные» | Постраничный OCR: `extract_pages_from_lopdf_document` извлекает каждую страницу, страницы с `count_alphabetic < 16` направляются в `ocr_specific_pages` (один рендер `pdftoppm -f/-l`, Tesseract только на нужные изображения). Гибридный fallback lopdf → pdf-extract (flat, page=1) → WinRT. `normalize_whitespace` применён ко всем экстракторам. | `extractor.rs:110` (`extract_text_with_pages`), `:173` (`extract_pdf_text_with_pages`), `:216` (`extract_pages_from_lopdf_document`), `:307` (`ocr_specific_pages`), `:1038` (`normalize_whitespace`) | ✅ unit-тесты компилируются; ❌ реальный смешанный PDF не тестировался |
| 4 | «последовательный вывод результатов поиска на уровне ядра CLI» | `search_files_streaming(files, query, on_result)` — callback вызывается сразу после завершения каждого файла; `search_files` стал пакетной обёрткой. CLI `print_search` печатает заголовок, затем каждый файл+вхождения в callback, затем итог. | `search.rs:31` (`search_files`), `:44` (`search_files_streaming`), `main.rs:68` (`print_search`) | ✅ 3 unit-теста + CLI smoke подтверждён |
| 5 | GTK-паритет воркера (`SearchResultFound`) | `WorkerEvent::SearchResultFound { result, completed, total }` добавлен в `actions.rs`. `worker.rs` шлёт его на каждый завершённый файл; `SearchCompleted.results` теперь `Vec::new()` (всё уже стримнуто). `app.rs::search_result_found` дедуплицирует по `relative_path`, мержит occurrences; `search_completed` не затирает streaming-результаты пустым batch. | `actions.rs:32`, `worker.rs:252`, `app.rs:298`, `app_win.rs:664` | ✅ компилируется (`windows-desktop`); ❌ GTK `--features desktop` не собирался |
| 6 | Открытие PDF + `ShellExecuteW code 2` | `open_file_at_page(path, page)` всегда передаёт чистый канонизированный путь в `win32::open_with_default_app`; `#page=N` фрагмент НЕ передаётся в `ShellExecuteW`. Номер страницы логируется на `debug` и показывается в UI. `/api/open-file` парсит body как `path|page`. | `app_win.rs:499` (`open_file_at_page`), `:1231` (`/api/open-file` парсинг) | ✅ компилируется; ❌ ручной запуск открытия PDF не выполнен |
| 7 (бонус) | Единый источник правды для расширений | `model/constants.rs`: `ALL_SUPPORTED_EXTENSIONS`, `TEXT_EXTENSIONS`, `DEFAULT_EXTENSIONS`. Scanner импортирует `DEFAULT_EXTENSIONS` и реэкспортирует `ALL_SUPPORTED_EXTENSIONS as UI_SUPPORTED_EXTENSIONS`. Extractor импортирует `TEXT_EXTENSIONS`. `/api/set-extensions` валидирует против `ALL_SUPPORTED_EXTENSIONS` (lowercase/dedup/без точки), без env-var. | `constants.rs`, `scanner.rs:5,10`, `extractor.rs:17`, `app_win.rs:1336` (`/api/set-extensions`) | ✅ компилируется; ❌ ручной UI-тест не выполнен |
| 8 (бонус) | `WorkerCommand::Scan` с явными расширениями | `Scan { root, extensions: Vec<String> }`; оба вызова в `app.rs` и все в `app_win.rs` передают `ui.state.scan_extensions.clone()`. Scanner использует `scan_directory_with_extensions(root, &extensions)`. Env-var `DOCUMENT_SEARCHER_EXTENSIONS` остаётся только legacy CLI-путём. | `actions.rs:6`, `worker.rs:69`, `app.rs:477,707`, `app_win.rs:762,1447`, `scanner.rs:60` | ✅ unit-тесты worker обновлены |

---

## 5. Архитектура и контракты

### 5.1 Дерево модулей (`src/src/`)

```
src/src/
├── main.rs            — точка входа, CLI-парсер (scan/search/gui/--ext), cfg-гейтинг UI
├── lib.rs             — re-export model
├── app.rs             — Linux GTK/relm4 UI (фича `desktop`); НЕ собирался в этой сессии
├── app_win.rs         — кросс-платформенный UI (Win32 + Linux CLI-фоллбэки), HTTP-сервер, JS/CSS
└── model/
    ├── constants.rs   [НОВЫЙ] — ALL_SUPPORTED_EXTENSIONS, TEXT_EXTENSIONS, DEFAULT_EXTENSIONS
    ├── state.rs       — FileEntry, ExtractedPage [НОВЫЙ], SearchOccurrence (+page), SearchMatch, AppState (+scan_extensions)
    ├── actions.rs     — WorkerCommand::Scan (+extensions), WorkerEvent (+SearchResultFound)
    ├── scanner.rs     — walkdir-обход, фильтр расширений, MAX_FILE_SIZE=50MiB
    ├── extractor.rs   — 3-уровневый PDF-экстрактор с постраничным OCR, кэши pages_cache
    ├── search.rs      — search_files_streaming + пакетная обёртка search_files
    └── worker.rs      — tokio-воркер, JoinSet, streaming результатов, отмена, debounce
```

### 5.2 Ключевые сигнатуры (источник правды для продолжения)

**`model/constants.rs`** — единый источник правды для расширений:
```rust
pub const ALL_SUPPORTED_EXTENSIONS: &[&str] = &[
    "docx", "pdf", "txt", "md", "rs", "toml", "json", "xml", "html", "css", "js", "ts",
];
pub const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "md", "rs", "toml", "json", "xml", "html", "css", "js", "ts",
];
pub const DEFAULT_EXTENSIONS: &[&str] = &["docx", "pdf"];
```

**`model/state.rs`** — новые/изменённые типы:
```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedPage {
    pub number: u32,            // 1-indexed
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchOccurrence {
    pub before: String,
    pub matched: String,
    pub after: String,
    pub snippet: String,
    pub page: Option<u32>,      // [НОВОЕ] None для plain-text
}

pub struct AppState {
    // ...существующие поля...
    pub scan_extensions: Vec<String>,  // [НОВОЕ] дефолт ["docx","pdf"]
}
```

**`model/actions.rs`** — изменённые команды/события:
```rust
pub enum WorkerCommand {
    Scan { root: PathBuf, extensions: Vec<String> },   // [ИЗМЕНЕНО]
    QueryChanged { query: String },
    Shutdown,
}

pub enum WorkerEvent {
    ScanStarted { root: String },
    ScanCompleted { files: Vec<FileEntry>, oversized_files: usize, unreadable_entries: usize },
    ScanError(String),
    SearchStarted { query: String },
    SearchResultFound {                                   // [НОВОЕ]
        result: SearchMatch,
        completed: usize,
        total: usize,
    },
    SearchProgress { completed: usize, total: usize },
    SearchCompleted { query: String, results: Vec<SearchMatch> }, // results теперь Vec::new()
    SearchCancelled { query: String },
}
```

**`model/search.rs`** — streaming-ядро:
```rust
pub fn search_files(files: &[FileEntry], query: &str)
    -> anyhow::Result<Vec<SearchMatch>>;

pub fn search_files_streaming<F>(
    files: &[FileEntry],
    query: &str,
    mut on_result: F,
) -> anyhow::Result<usize>
where
    F: FnMut(SearchMatch) -> anyhow::Result<()>;
```
- `search_files_streaming` извлекает `Vec<ExtractedPage>` через `extract_text_with_pages`,
  ищет `find_occurrences_with_context` по каждой странице, проставляет `occ.page = Some(page.number)`,
  и вызывает `on_result` для файлов хотя бы с одним вхождением. Ошибка извлечения
  одного файла → `tracing::warn!` и `continue`, не останавливая поиск.

**`model/extractor.rs`** — пайплайн с постраничным OCR:
```rust
pub fn extract_text(path: &Path) -> Result<String>;                 // legacy, кэш TEXT_CACHE
pub fn extract_text_with_pages(path: &Path) -> Result<Vec<ExtractedPage>>;  // [НОВОЕ], кэш PAGES_CACHE

fn extract_pdf_text_with_pages(path: &Path) -> Result<Vec<ExtractedPage>> {
    // lopdf::Document::load(path)
    //   Ok(doc)  -> extract_pages_from_lopdf_document(&doc, path)
    //   Err(_)   -> pdf-extract flat (page=1) если alpha>=32, иначе extract_scanned_pdf_text_with_pages
}

fn extract_pages_from_lopdf_document(doc: &Document, path: &Path) -> Result<Vec<ExtractedPage>> {
    const PAGE_TEXT_ALPHA_THRESHOLD: usize = 16;
    // 1. Для каждой страницы: extract_single_page_text; если count_alphabetic < 16 -> ocr_candidate
    // 2. Если есть ocr_candidates: один pdf-extract pass; если total_lopdf_alpha < 32 -> вернуть flat
    // 3. ocr_specific_pages(path, &candidates) -> заменить текст слабых страниц, если OCR лучше
}

#[cfg(any(feature = "ocr", feature = "windows-ocr"))]
fn ocr_specific_pages(path: &Path, page_numbers: &[u32]) -> Result<Vec<ExtractedPage>>;
#[cfg(not(any(feature = "ocr", feature = "windows-ocr")))]
fn ocr_specific_pages(_path: &Path, _page_numbers: &[u32]) -> Result<Vec<ExtractedPage>>; // Ok(Vec::new())

fn extract_single_page_text(doc: &Document, page_num: u32, page_id: ObjectId) -> String;
fn normalize_whitespace(text: &str) -> String;
fn count_alphabetic(text: &str) -> usize;
```
Кэши: `TEXT_CACHE` (flat) и `PAGES_CACHE` (постраничный) — оба `OnceLock<RwLock<HashMap<CacheKey, _>>>`
по ключу `(path, size, modified)`, лимит `MAX_CACHED_TEXT_BYTES = 8 MiB`, `MAX_CACHE_ENTRIES` с очисткой.

**`model/worker.rs`** — streaming-воркер:
```rust
impl Worker {
    async fn start_scan(&mut self, root: PathBuf, extensions: Vec<String>, events);
    async fn start_search(&mut self, query: String, events);
    // start_search: tokio::select! debounce 300мс -> JoinSet per-file
    //   каждый completed файл -> WorkerEvent::SearchResultFound { result, completed, total }
    //   финал -> WorkerEvent::SearchCompleted { query, results: Vec::new() }
}
```

**`main.rs::print_search`** — CLI streaming:
```rust
fn print_search(root: &Path, query: &str, extensions: Option<&[String]>) -> anyhow::Result<()> {
    let report = scan (with extensions or report);
    println!("Searching {query:?} across {} files…", report.files.len());
    let mut stdout = BufWriter::new(io::stdout());
    let reported = search_files_streaming(&report.files, query, |result| {
        writeln!(stdout, "\n{}", result.file_relative_path)?;
        for occurrence in &result.occurrences {
            let page_info = occurrence.page.map(|p| format!("  (page {p})")).unwrap_or_default();
            writeln!(stdout, "{page_info}  {}", occurrence.snippet)?;
        }
        stdout.flush()?;
        Ok(())
    })?;
    stdout.flush()?;
    println!("\nMatches for {query:?}: {reported} files");
}
```

**`app_win.rs`** — Windows UI контракты:
```rust
fn open_file_at_page(path: &Path, page: Option<u32>) -> anyhow::Result<()> {
    // clean = normalize_platform_path(path); проверка is_file;
    // page логируется на debug; НЕ передаётся в ShellExecuteW;
    // win32::open_with_default_app(&clean)
}

// HTTP endpoints:
// POST /api/open-file        body: "path" или "path|page"  -> open_file_at_page
// POST /api/set-extensions   body: "docx,pdf,txt"          -> валидация, scan_extensions, rescan
// GET  /api/state            -> JSON, вкл. "scanExtensions": [...], occurrence "page": N
```

### 5.3 Поток данных

```
CLI:  main::print_search
        -> scan_directory_with_extensions
        -> search_files_streaming (callback per file)
        -> writeln!(stdout) немедленно

GUI (Windows): app_win::run
        Controller.bridge.send(WorkerCommand::Scan{root, extensions})
        -> Worker::start_scan -> scan_directory_with_extensions
        -> WorkerEvent::ScanCompleted -> WinUiState.found_files
        Controller.bridge.send(WorkerCommand::QueryChanged)
        -> Worker::start_search (debounce 300ms, JoinSet)
        -> WorkerEvent::SearchResultFound per file -> WinUiState.search_result_found (dedup/merge)
        -> WorkerEvent::SearchCompleted (results=Vec::new) -> search_completed (не затирает)
        HTTP /api/state polled клиентом -> JSON -> JS render

GUI (Linux GTK): app.rs::build_ui — тот же WorkerEvent контракт,
        Ui::search_result_found / search_completed с той же логикой merge.
```

### 5.4 PDF-пайплайн (4-уровневый)

```
extract_text_with_pages(path)
├── PDF? -> extract_pdf_text_with_pages(path)
│   ├── Document::load(path) Ok(doc)
│   │   └── extract_pages_from_lopdf_document(doc, path)
│   │       ├── per-page extract_single_page_text (builtin + custom, Cyrillic-aware)
│   │       ├── страницы с count_alphabetic < 16 -> ocr_candidates
│   │       ├── pdf-extract flat pass (если total_lopdf_alpha < 32 -> вернуть flat page=1)
│   │       └── ocr_specific_pages(path, &candidates) [pdftoppm -f/-l + tesseract]
│   └── Document::load Err
│       ├── pdf_extract::extract_text (catch_unwind) -> flat если alpha>=32
│       └── extract_scanned_pdf_text_with_pages(path)
│           ├── pdftoppm + tesseract per page (точные номера из filename stem)
│           └── Windows WinRT fallback (consolidated page=1)
└── не-PDF -> extract_text -> vec![ExtractedPage{number:1, text}]
```
`normalize_whitespace` применяется к: plain text, DOCX, builtin lopdf, custom lopdf,
pdf-extract, OCR (tesseract), WinRT. Не ломает границы страниц (постранично) и
UTF-8 offsets (работает по `char`, не по байтам).

---

## 6. Диффы по модулям (сводка)

Полный дифф: `+1161 / −296` по 11 файлам + 5 untracked.

| Файл | +/− | Суть изменений |
|---|---|---|
| `src/src/model/constants.rs` | [новый] | Единственный источник расширений. |
| `src/src/model/mod.rs` | +1 | `pub mod constants;` |
| `src/src/model/scanner.rs` | +2/−3 | Импорт `DEFAULT_EXTENSIONS`, реэкспорт `ALL_SUPPORTED_EXTENSIONS as UI_SUPPORTED_EXTENSIONS`; удалён локальный `DEFAULT_EXTENSIONS`. |
| `src/src/model/state.rs` | +13/−0 | `ExtractedPage`, `SearchOccurrence.page`, `AppState.scan_extensions`. |
| `src/src/model/actions.rs` | +13/−2 | `Scan.extensions`, `SearchResultFound`, doc-комментарии. |
| `src/src/model/search.rs` | +99/−17 | `search_files_streaming`, `search_files` как обёртка, per-page occurrences, 3 unit-теста. |
| `src/src/model/extractor.rs` | +451/−235 | `extract_text_with_pages` + `PAGES_CACHE`, постраничный OCR (`extract_pages_from_lopdf_document`, `ocr_specific_pages`, `extract_scanned_pdf_text_with_pages`), `normalize_whitespace` везде, удалён `diag_pdf_pipeline` ignored-тест. |
| `src/src/model/worker.rs` | +36/−14 | `start_scan(extensions)`, streaming `SearchResultFound`, `SearchCompleted.results=Vec::new()`, тесты обновлены под `extensions`. |
| `src/src/main.rs` | +21/−8 | `print_search` через `search_files_streaming` + `BufWriter`, page-info в выводе. |
| `src/src/app.rs` | +37/−4 | `search_result_found` (dedup/merge), `search_completed` не затирает, оба `Scan` с `extensions`, filter `warn`, dispatch `SearchResultFound`. |
| `src/src/app_win.rs` | +424/−23 | `open_file_at_page`, `WinUiState::search_result_found`, `/api/open-file` (path\|page), `/api/set-extensions` (валидация), `serialize_state_json` (`scanExtensions`, `page`), hamburger-dropdown CSS/HTML/JS, filter `warn`, startup `debug!`. |
| `prompts-in-between-sessions.txt` | +25/−0 | Добавлены промпты между сессиями (не код). |
| **Untracked** | | `docs/DEVELOPMENT_HANDOFF.md`, `docs/NEXT_SESSION_PROMPT.md`, `last-implementation-plan.md`, `src/src/model/constants.rs`, `work-for-9-11-2026.txt` |

---

## 7. Не реализовано / backlog (на следующую сессию)

### 7.1 Тестовое покрытие (P1)
- **Синтетические PDF-фикстуры через `lopdf` в Rust-тесте** (текстовый PDF, скан-симуляция image-only, смешанный PDF = текстовая страница + image-only страница). Цель — покрыть `extract_pages_from_lopdf_document` и `ocr_specific_pages` без ручных файлов. Тесты OCR запускать только при доступных `pdftoppm`/`tesseract` или WinRT, иначе `#[ignore]` с явным сообщением.
- **Environment-verification PowerShell-скрипт**: проверить наличие `pdftoppm.exe` и `tesseract.exe` (и `DOCUMENT_SEARCHER_OCR_LANG`), сообщить OCR-readiness.

### 7.2 Среда (P1, не блокирует код)
- **Linux/ALT Linux VM**: `cargo build --release --features desktop`, `cargo check --all-targets --features desktop`, GUI smoke, CSS lookup из embedded resource, portal folder chooser, GSettings schema discovery.
- **Yandex Browser**: проверить поведение `file:///...#page=N` через явный запуск browser command, если deep-link строго требуется. Текущая реализация **не обещает** universal deep-link — открывает чистый путь, страница показывается в UI для ручной навигации.
- **OCR на реальных PDF**: запустить WinRT path и/или Tesseract на реальном смешанном PDF, сверить номера страниц и нормализацию пробелов.

### 7.3 Ручная проверка Windows UI (P2)
- Hamburger-dropdown: открытие/закрытие, click-outside, Escape, сохранение выбора после `/api/state`, пересканирование.
- Streaming results: постепенное появление карточек, dedup/merge, не затирается на `SearchCompleted`.
- Открытие PDF: чистый путь, fallback при недоступности page URI, whitelist индекса.
- `/api/set-extensions`: валидация (lowercase/dedup/без точки/только supported), 400 на пустой список.

### 7.4 Возможные улучшения (P3)
- GTK `app.rs`: добавить `gtk::MenuButton`/`Popover` с checkbox-элементами расширений (план пункт 2 упоминал, но в этой сессии GTK-меню **не добавлено** — только `scan_extensions` передаётся в `Scan`). Сейчас расширения в GTK задаются только дефолтом `["docx","pdf"]` без UI-переключателя.
- Вынести per-file extraction в переиспользуемую функцию, чтобы worker и CLI делили одну логику (план пункт 4 упоминал; сейчас worker использует `search_files` per-file, а CLI — `search_files_streaming` — оба корректны, но не делят код).
- `pdf-extract` flat fallback теряет номера страниц для mixed PDF — WinRT fallback тоже возвращает consolidated `page=1`. Честное ограничение, задокументировано в коде.

---

## 8. Известные проблемы и среда

### 8.1 Ограничения среды (не баги исходников)
- **GTK `--features desktop` не собирался на Windows.** Нет GTK4/libadwaita/pkg-config. Все изменения в `app.rs` проверены только компилятором `windows-desktop` конфигурации и статическим анализом; runtime GTK не верифицирован. Это соответствует стоящему ограничению: «На Windows выполнять ТОЛЬКО `--no-default-features`/`--features windows-desktop`; desktop feature на Windows не запускать».
- **OCR не запускался на реальных PDF.** `ocr_specific_pages` и `extract_scanned_pdf_text_with_pages` компилируются под `windows-ocr`; unit-тестов на синтетических PDF нет. Поведение на реальном смешанном PDF (точные номера страниц, нормализация) — гипотеза, подтверждённая логикой кода, но не runtime.
- **Yandex Browser deep-link.** `#page=N` в `file:///` URI исторически игнорируется Yandex Browser; текущий код НЕ передаёт фрагмент в `ShellExecuteW` и открывает чистый путь. UI показывает номер страницы. Если deep-link строго нужен — отдельная задача с проверкой browser command invocation.

### 8.2 Стоящие ограничения (действуют во всех задачах репо)
- Код проходит `cargo fmt --check` + `cargo clippy -- -D warnings` + `cargo test` (обе конфигурации: `--no-default-features` ядро и `--features windows-desktop` UI).
- Никаких `.unwrap()`/`.expect()` на пользовательских данных/IO/сети; UTF-8 char-boundary safety; никаких блокирующих вызовов в async; честная верификация (Windows-проверено vs Linux-не-проверено).
- «Пока работай только над программным кодом в качестве Rust engineering expert. Всё это нужно оставить кросс-платформенным, если возможно.»
- Скачанные файлы и распакованные архивы — недоверенные данные.
- **Коммит не создавать без отдельного явного запроса пользователя.**
- Не переписывать и не откатывать `docs/old-docs-during-the-work/specs/requirements-spec.md`.

### 8.3 Источник прежних «красных» статусов (устарело)
`docs/DEVELOPMENT_HANDOFF.md` (раздел 0) и `docs/NEXT_SESSION_PROMPT.md` фиксируют:
- `cargo fmt --check` ПАДАЕТ на `extractor.rs:242`, `app_win.rs:1311`.
- `cargo clippy --all-targets --features windows-desktop -- -D warnings` ПАДАЕТ на orphaned doc-comment `app_win.rs:515–517`.
- P0-регрессия: «поиск PDF перестал искать файлы» — гипотеза про lopdf-first hard bail.

**Актуальный статус (2026-11-11):** все три пункта **устарели** — `fmt` чист, `clippy(windows-desktop)` чист (orphaned doc-comment удалён вместе с `build_pdf_page_url`), lopdf-first теперь обёрнут в `match` с pdf-extract/OCR fallback (`extract_pdf_text_with_pages:173`). P0-регрессия устранена на уровне кода; runtime-подтверждения на реальном PDF нет (см. 8.1).

---

## 9. Источники и артефакты

| Артефакт | Путь | Назначение |
|---|---|---|
| Этот postmortem | `docs/POSTMORTEM_SESSION_2026_11_11.md` | Продолжение с нуля |
| Предыдущий handoff | `docs/DEVELOPMENT_HANDOFF.md` | Контекст сессии 2026-11-09 (часть утверждений устарела — см. 8.3) |
| Промпт для чистой сессии | `docs/NEXT_SESSION_PROMPT.md` | Инструкции старта (часть устарела) |
| Утверждённый план | `~/.claude/plans/woolly-plotting-spark.md` (копия `last-implementation-plan.md`) | Источник правды по объёму работы |
| Промпты между сессиями | `prompts-in-between-sessions.txt` | Последовательность пользовательских запросов |
| Полные транскрипты | `C:\Users\максим\.claude\projects\C--Users--------Documents-programming-projects-document-searcher\` | JSONL: `3e6ba484-…jsonl` (предыдущая), `3f27602a-…jsonl` (эта) |
| Memory | `~/.claude/projects/.../memory/` | `MEMORY.md` + `development-handoff-doc.md`, `rust-architect-role-and-constraints.md` |

---

## 10. Быстрый старт для следующей сессии

1. Прочитай этот файл целиком.
2. Из `src/` запусти верификацию (должна быть зелёной):
   ```
   cargo fmt --check
   cargo clippy --lib --tests --no-default-features -- -D warnings
   cargo clippy --all-targets --features windows-desktop -- -D warnings
   cargo test --lib --no-default-features
   cargo test --bins --features windows-desktop
   ```
3. Если всё зелёное — реализуй backlog по приоритетам раздела 7:
   - P1: синтетические PDF-фикстуры (lopdf) + env-verification скрипт.
   - P1: Linux/ALT Linux VM проверки (`--features desktop`, OCR на реальном PDF).
   - P2: ручная проверка Windows UI.
   - P3: GTK `MenuButton`/`Popover` для расширений; вынести per-file extraction в общую функцию.
4. Не создавай коммит без явного запроса пользователя.
5. Помни про стоящие ограничения (раздел 8.2) и честную верификацию (Windows vs Linux).

---

*Составлено 2026-11-11. Все верификационные статусы сверены с кодом вживую в этой сессии.*
