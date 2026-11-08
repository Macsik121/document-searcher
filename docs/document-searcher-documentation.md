# Единая техническая спецификация и архитектура системы (SRS & System Design)

**Проект:** Document Searcher  
**Версия архитектуры:** 2.0 (Unified Cross-Platform Edition)  
**Целевые платформы:** Windows 10/11, ALT Linux (p10/p11), Ubuntu Linux (22.04 / 24.04 LTS)  
**Язык и тулчейн:** Rust Stable (`1.78.0`, Edition 2021)  

---

## 1. Назначение и концепция системы

**Document Searcher** — это высокопроизводительное настольное и консольное (CLI + GUI) приложение для глубокого контекстного поиска подстрок внутри локальных документов форматов **`.docx`** и **`.pdf`** (с возможностью расширения списка форматов через параметры запуска).

Приложение спроектировано по принципу **Zero-C-Build-Dependencies**: оно не требует установки тяжёлых компиляторов C/C++ или `*-devel` пакетов графических тулкитов при сборке из исходного кода и компилируется единой командой `cargo build --release` на Windows, ALT Linux и Ubuntu.

---

## 2. Функциональные требования (FR)

### FR-1: Управление директориями и запуск
* **FR-1.1:** Приложение поддерживает как графический режим (GUI), так и консольный режим (`scan`, `search`) в едином исполняемом файле `document-searcher`.
* **FR-1.2:** При запуске без аргументов приоритетно открывается последняя сохранённая папка из конфигурационного файла (`settings.conf`) либо текущая рабочая директория (`current_dir`).
* **FR-1.3:** При передаче пути к папке через аргумент командной строки (`document-searcher <путь>` или `document-searcher gui <путь>`) приложение немедленно открывает и индексирует указанную директорию.
* **FR-1.4:** Пользователь может в любой момент сменить папку через нативный системный диалог выбора директории (Win32 `SHBrowseForFolderW` на Windows; `zenity` / `kdialog` на ALT Linux и Ubuntu). Отмена диалога сохраняет текущий индекс и результаты поиска неизменными.

### FR-2: Индексация файлов (`scanner.rs`)
* **FR-2.1:** Рекурсивный обход дерева каталогов выполняется через `WalkDir` без перехода по символическим ссылкам (`follow_links(false)`) во избежание циклических петель.
* **FR-2.2:** Скрытые файлы и директории (начинающиеся с `.`) автоматически игнорируются на любом уровне вложенности.
* **FR-2.3:** По умолчанию индексируются **только файлы `.docx` и `.pdf`** (регистронезависимо). Пользователь может переопределить список расширений через флаг `--ext <список>` или переменную окружения `DOCUMENT_SEARCHER_EXTENSIONS`.
* **FR-2.4:** Файлы размером более **50 MiB** (`52 428 800` байт) пропускаются с увеличением счётчика `oversized_files`. Нечитаемые записи фиксируются в счётчике `unreadable_entries`.
* **FR-2.5:** Все относительные пути нормализуются к единому разделителю `/` и сортируются лексикографически. Расширенные пути Windows (`\\?\C:\...` и `\\?\UNC\server\share\...`) нормализуются функцией `normalize_platform_path`.

### FR-3: Многоуровневое извлечение текста и кэширование (`extractor.rs`)
* **FR-3.1 (In-Memory Кэш `TEXT_CACHE`):** Извлечённый текст каждого файла кэшируется в потокобезопасном хранилище `OnceLock<RwLock<HashMap<CacheKey, CacheValue>>>` по составному ключу `(PathBuf, размер_файла, время_модификации)`. Повторные поисковые запросы читают текст из ОЗУ за микросекунды без повторного обращения к диску. Ошибки повреждённых файлов также кэшируются, чтобы не парсить сбойные файлы повторно.
* **FR-3.2 (Извлечение `.docx`):** Выполняется путём потокового чтения `word/document.xml` из ZIP-архива через `quick-xml` с сохранением абзацев (`w:p`), переносов (`w:br`) и табуляций (`w:tab`).
* **FR-3.3 (Трёхуровневый конвейер `.pdf`):**
  1. **Уровень 1 — Постраничный декодер `lopdf` с поддержкой кириллицы:** Разбирает PDF постранично, декодируя таблицы шрифтов `/Encoding -> /Differences` (включая глифы `/uni0410`..`/uni044F`, `/afii10017`..`/afii10097`, `.cap`-глифы), потоки `/ToUnicode` (`beginbfchar` и `beginbfrange`), `UTF-16BE` и однобайтовую кириллицу `CP1251`.
  2. **Уровень 2 — Изолированный `pdf-extract`:** Запускается внутри `std::panic::catch_unwind` (защита от паник на сложных цветовых функциях PDF, таких как *Function Type 4*). Выбирается результат с наибольшим числом буквенных символов.
  3. **Уровень 3 — OCR для отсканированных PDF без текстового слоя:** Если текстовый слой пуст, вызывается рендеринг страниц через `pdftoppm` + `tesseract` (`eng+rus` с автоматическим откатом на `eng`), а на Windows при отсутствии внешних утилит — встроенный системный движок **Windows WinRT (`Windows.Data.Pdf` + `Windows.Media.Ocr`)**.

### FR-4: Регистронезависимый поиск и многопоточность (`search.rs`, `worker.rs`)
* **FR-4.1 (Unicode Case-Folding):** Поиск сопоставляет подстроку без учёта регистра через структуру `FoldedText`, сохраняющую точные байтовые границы (`source_spans`) каждого символа исходного текста UTF-8.
* **FR-4.2 (Контекстные сниппеты):** Для каждого вхождения формируется структура `SearchOccurrence` с контекстом **до 20 символов до совпадения** (`before`), **точной найденной подстрокой** (`matched`) и **до 20 символов после совпадения** (`after`).
* **FR-4.3 (Debounce и отмена):** Поиск запускается с задержкой **300 мс** после последнего ввода символа. Новый ввод немедленно отменяет текущий поиск через `CancellationToken` и `join_set.abort_all()`.
* **FR-4.4 (Параллельный пул `JoinSet` + `Semaphore`):** Извлечение и поиск выполняются параллельно на всех доступных ядрах CPU (`available_parallelism()`, от 2 до 8 потоков одновременно) с детерминированной итоговой сортировкой результатов.

### FR-5: Графический интерфейс, пагинация и безопасность (`app.rs` / `app_win.rs`)
* **FR-5.1 (Разделение экрана 50/50):** Левая панель отображает текущий путь, количество найденных файлов, прокручиваемый список файлов и кнопку смены папки. Правая панель содержит поле поиска, индикатор состояния (спиннер / зелёная галочка / ошибка) и карточки результатов с жёлтым выделением совпадений (`<mark>`).
* **FR-5.2 (Кумулятивная пагинация):** Результаты выдаются порциями по **100 документов**. Кнопка `"Show more"` догружает следующие 100 результатов в список без удаления предыдущих.
* **FR-5.3 (Безопасное открытие файлов):** Клик по файлу открывает его в программе ОС по умолчанию (Win32 `ShellExecuteW` на Windows; `xdg-open` на ALT Linux / Ubuntu) только после проверки принадлежности пути к белому списку текущего индекса (`is_indexed_file`) и проверки сессионного CSRF-токена (`X-Document-Searcher-Token`).

---

## 3. Нефункциональные требования (NFR)

* **NFR-1 (Отзывчивость UI):** Поток управления состоянием UI никогда не блокируется файловым вводом-выводом или парсингом PDF.
* **NFR-2 (Отказоустойчивость):** Паника парсера, повреждённый ZIP/PDF или ошибка чтения одного файла перехватываются на уровне файла (`catch_unwind` + `Result`), логируются через `tracing::warn!` и никогда не прерывают пакетный поиск по остальным файлам.
* **NFR-3 (Автономность):** Вся вёрстка, стили и скрипты встроены в исполняемый файл (`INDEX_HTML`) и работают на 100% офлайн без внешних CDN.

---

## 4. BPMN 2.0: Сквозной бизнес-процесс работы системы

Диаграмма ниже описывает полный цикл работы приложения в разрезе трёх дорожек (Swimlanes): **Пользователь**, **UI & Synchronized Controller** и **Async Tokio Worker & Extraction Pool**.

```mermaid
flowchart TB
    subgraph Lane_User["👤 Дорожка 1: Пользователь"]
        U_Start(["Запуск приложения\n(CLI или GUI)"])
        U_ChangeFolder["Нажатие 'Change folder'\nВыбор папки в диалоге ОС"]
        U_TypeQuery["Ввод поискового запроса\nв строку поиска"]
        U_ClickMore["Нажатие 'Show more'\n(Пагинация +100)"]
        U_ClickFile["Клик по файлу\n(Открытие документа)"]
    end

    subgraph Lane_Controller["🖥️ Дорожка 2: UI, HttpWorkerPool (4 потока) & Монитор Controller"]
        C_Init["Инициализация Controller\nЧтение settings.conf / CLI аргумента\nГенерация Session Token"]
        C_Dialog["Открытие системного диалога папки\n(Win32 SHBrowseForFolderW / Linux zenity)"]
        C_DialogCheck{"Папка\nвыбрана?"}
        C_SavePath["Сохранение пути в settings.conf\nАтомарный сброс состояния и отправка Scan"]
        C_QueryGate{"Проверка X-Query-Seq\nseq >= last_query_seq?"}
        C_AtomicSubmit["Атомарно под Mutex:\nОбновление search_query\nОтправка QueryChanged в Worker"]
        C_RenderUI["Отрисовка состояния в UI:\nСпиннер / Прогресс / Карточки с <mark>"]
        C_LoadMore["Вызов state.load_more()\nРасширение среза +100"]
        C_SecurityCheck{"Токен валиден И\nфайл в found_files?"}
        C_OpenOS["Открытие в ОС по умолчанию\n(Win32 ShellExecuteW / Linux xdg-open)"]
    end

    subgraph Lane_Worker["⚙️ Дорожка 3: Tokio Worker, JoinSet Pool & 3-Tier Extractor"]
        W_Scan["Обход WalkDir (только .docx и .pdf)\nЛимит 50 MiB, пропуск скрытых и симлинков"]
        W_ScanDone["Сортировка путей\nОтправка WorkerEvent::ScanCompleted"]
        W_CancelOld["Отмена предыдущего поиска:\nCancellationToken::cancel()\njoin_set.abort_all()"]
        W_Debounce{"Ожидание Debounce\n300 мс (или отмена)"}
        W_Parallel["Запуск параллельного пула JoinSet\nОграничение: Semaphore(CPU cores 2..8)"]
        W_CacheCheck{"Файл есть в\nTEXT_CACHE?"}
        W_Extract["3-уровневое извлечение:\n1. DOCX XML / lopdf (кириллица uni04XX, ToUnicode, CP1251)\n2. pdf-extract (внутри catch_unwind)\n3. OCR Fallback (Tesseract / WinRT)"]
        W_SaveCache["Запись результата/ошибки\nв TEXT_CACHE"]
        W_Match["Unicode Case-Folded поиск (FoldedText)\nФормирование контекста ±20 символов"]
        W_Complete["Детерминированная сортировка\nОтправка SearchCompleted"]
    end

    U_Start --> C_Init --> W_Scan --> W_ScanDone --> C_RenderUI
    U_ChangeFolder --> C_Dialog --> C_DialogCheck
    C_DialogCheck -- "Отмена" --> C_RenderUI
    C_DialogCheck -- "Выбрана" --> C_SavePath --> W_Scan

    U_TypeQuery --> C_QueryGate
    C_QueryGate -- "Устаревший пакет (отбросить)" --> C_RenderUI
    C_QueryGate -- "Актуальный" --> C_AtomicSubmit --> W_CancelOld --> W_Debounce
    W_Debounce -- "Новый ввод до 300 мс" --> W_CancelOld
    W_Debounce -- "300 мс истекли" --> W_Parallel --> W_CacheCheck
    W_CacheCheck -- "Cache Hit (ОЗУ)" --> W_Match
    W_CacheCheck -- "Cache Miss (Диск)" --> W_Extract --> W_SaveCache --> W_Match
    W_Match --> W_Complete --> C_RenderUI

    U_ClickMore --> C_LoadMore --> C_RenderUI
    U_ClickFile --> C_SecurityCheck
    C_SecurityCheck -- "Да" --> C_OpenOS
    C_SecurityCheck -- "Нет (403 Forbidden)" --> C_RenderUI
```

---

## 5. UML-диаграммы архитектуры

### 5.1. UML Component Diagram (Диаграмма компонентов и платформенных адаптеров)

```mermaid
flowchart TB
    subgraph Presentation["Слой представления и маршрутизации"]
        Main["src/main.rs\nУниверсальная точка входа (GUI + CLI)"]
        WebUI["Embedded Offline UI (INDEX_HTML)\nМакет app-design.html + AbortController"]
        HttpPool["HttpWorkerPool\n4 постоянных потока + sync_channel(32)"]
    end

    subgraph Synchronization["Слой синхронизации (Monitor Boundary)"]
        Ctrl["Controller (Arc<Mutex<Controller>>)\nЕдиная точка сериализации стейта и команд"]
        UiState["UiState\nКонечный автомат UI + проверка белого списка"]
        Bridge["RuntimeBridge\nУправляемый жизненный цикл потока Tokio + Drop Shutdown"]
    end

    subgraph AsyncCore["Асинхронное ядро (Feature-Free Domain Core)"]
        Worker["model/worker.rs\nTokio Worker (Debounce 300ms, CancellationToken, JoinSet)"]
        Scanner["model/scanner.rs\nWalkDir (.docx, .pdf, <=50 MiB)"]
        Search["model/search.rs\nFoldedText Unicode Matcher (±20 символов)"]
        Extractor["model/extractor.rs\nTEXT_CACHE + lopdf Кириллица + pdf-extract + OCR"]
        State["model/state.rs & actions.rs\nAppState, WorkerCommand, WorkerEvent"]
    end

    subgraph PlatformAdapter["Кроссплатформенный адаптер ОС (platform)"]
        WinAdapter["#[cfg(target_os = 'windows')]\nSHBrowseForFolderW, ShellExecuteW,\nWinRT OCR, Tesseract/Poppler CLI"]
        LinuxAdapter["#[cfg(target_os = 'linux')]\nALT Linux / Ubuntu:\nzenity / kdialog, xdg-open,\nTesseract + pdftoppm CLI"]
    end

    Main --> HttpPool
    Main --> Scanner
    Main --> Search
    WebUI <-->|"HTTP JSON / X-Document-Searcher-Token"| HttpPool
    HttpPool --> Ctrl
    Ctrl --> UiState
    Ctrl --> Bridge
    Bridge <-->|"mpsc::channel\nWorkerCommand / WorkerEvent"| Worker
    Worker --> Scanner
    Worker --> Search
    Search --> Extractor
    UiState --> State
    Worker --> State
    Ctrl --> PlatformAdapter
    Extractor --> PlatformAdapter
    PlatformAdapter --> WinAdapter
    PlatformAdapter --> LinuxAdapter
```

---

### 5.2. UML Class Diagram (Диаграмма классов и структур данных Rust)

```mermaid
classDiagram
    class Controller {
        -ui: UiState
        -bridge: RuntimeBridge
        +submit_query(query: String, seq: Option~u64~)
        +start_folder_scan(folder: PathBuf)
    }

    class UiState {
        +state: AppState
        +status_text: String
        +version: u64
        +last_query_seq: u64
        +folder_dialog_open: bool
        +is_indexed_file(candidate: &Path) bool
        +accept_query_edit(query: String, seq: Option~u64~) bool
        +scan_started()
        +scan_completed(files: Vec~FileEntry~, oversized: usize, unreadable: usize)
        +search_started(query: &str)
        +search_progress(completed: usize, total: usize)
        +search_completed(query: String, results: Vec~SearchMatch~)
        +load_more()
    }

    class RuntimeBridge {
        -commands: mpsc::Sender~WorkerCommand~
        -runtime_thread: Option~JoinHandle~
        +start() (RuntimeBridge, mpsc::Receiver~WorkerEvent~)
        +send(command: WorkerCommand)
    }

    class Worker {
        -files: Arc~RwLock~Vec~FileEntry~~~~
        -search_task: Option~JoinHandle~
        -search_cancel: Option~CancellationToken~
        -scan_task: Option~JoinHandle~
        -search_generation: Arc~AtomicU64~
        -scan_generation: Arc~AtomicU64~
        -extraction_slots: Arc~Semaphore~
        +run(commands: Receiver~WorkerCommand~, events: Sender~WorkerEvent~)
    }

    class AppState {
        +settings: AppSettings
        +current_path: PathBuf
        +found_files: Vec~FileEntry~
        +search_query: String
        +search_results: Vec~SearchMatch~
        +results_offset: usize
        +results_limit: usize
        +status: SearchStatus
        +error: Option~String~
        +skipped_oversized_files: usize
        +skipped_unreadable_entries: usize
        +set_scan_path(path: PathBuf)
        +get_visible_results() &[SearchMatch]
        +has_more_results() bool
        +load_more()
    }

    class FileEntry {
        +relative_path: String
        +absolute_path: PathBuf
    }

    class SearchMatch {
        +file_relative_path: String
        +file_absolute_path: PathBuf
        +contexts: Vec~String~
        +occurrences: Vec~SearchOccurrence~
    }

    class SearchOccurrence {
        +before: String
        +matched: String
        +after: String
        +snippet: String
    }

    class CacheKey {
        +path: PathBuf
        +len: u64
        +modified: Option~SystemTime~
    }

    class FontCharMap {
        +single_byte: HashMap~u8, String~
        +two_byte: HashMap~u16, String~
    }

    Controller *-- UiState
    Controller *-- RuntimeBridge
    UiState *-- AppState
    AppState "1" *-- "*" FileEntry
    AppState "1" *-- "*" SearchMatch
    SearchMatch "1" *-- "*" SearchOccurrence
    RuntimeBridge ..> Worker : commands / events
```

---

### 5.3. UML Sequence Diagram (Многопоточный поиск, отмена и работа кэша)

```mermaid
sequenceDiagram
    autonumber
    actor User as Пользователь
    participant UI as Окно UI (JS + AbortController)
    participant Pool as HttpWorkerPool (4 потока)
    participant Ctrl as Controller (Mutex)
    participant W as Tokio Worker
    participant JS as JoinSet + Semaphore(N ядер)
    participant Cache as TEXT_CACHE (RwLock)
    participant Ext as 3-Tier Extractor (lopdf / docx / OCR)

    User->>UI: Вводит "химия" (символ за символом)
    UI->>UI: AbortController.abort() старого запроса\nquerySeq += 1
    UI->>Pool: POST /api/query (X-Query-Seq: 10, body: "химия")
    Pool->>Ctrl: lock() -> submit_query("химия", Some(10))
    Ctrl->>Ctrl: Проверка 10 >= last_query_seq\nstatus = Searching ("Waiting for input…")
    Ctrl->>W: WorkerCommand::QueryChanged { query: "химия" }
    Ctrl-->>Pool: unlock()
    Pool-->>UI: 200 OK

    W->>W: cancel_search() (CancellationToken::cancel)\ngeneration += 1
    W->>W: Ожидание SEARCH_DEBOUNCE (300 мс)
    W-->>Ctrl: WorkerEvent::SearchStarted { query: "химия" }

    loop Параллельно для всех файлов (JoinSet до 8 потоков)
        W->>JS: spawn(file, query, semaphore_permit)
        JS->>Cache: read().get(CacheKey { path, len, mtime })
        alt Cache Hit (Уже в памяти)
            Cache-->>JS: Ok(Arc<String>) за ~0.001 мс
        else Cache Miss (Чтение с диска)
            JS->>Ext: extract_text_uncached(path)
            Ext-->>JS: Декодированный UTF-8 текст (lopdf / docx / OCR)
            JS->>Cache: write().insert(CacheKey, Ok(Arc<String>))
        end
        JS->>JS: FoldedText::new() -> поиск вхождений ±20 символов
        JS-->>W: Vec<SearchMatch>
        W-->>Ctrl: WorkerEvent::SearchProgress { completed, total }
    end

    W->>W: Сортировка результатов по relative_path
    W-->>Ctrl: WorkerEvent::SearchCompleted { query: "химия", results }
    UI->>Pool: GET /api/state
    Pool-->>UI: JSON (status: "success", results с подсветкой)
```

---

### 5.4. UML State Machine Diagram (Конечный автомат состояний `SearchStatus`)

```mermaid
stateDiagram-v2
    [*] --> Idle: Инициализация AppState

    Idle --> Scanning: WorkerCommand::Scan / ScanStarted
    Scanning --> Idle: ScanCompleted (N файлов проиндексировано)
    Scanning --> Error: ScanError (Папка недоступна)

    Scanning --> Scanning: QueryChanged(пустой) [Игнорируется, не сбивает Scanning]

    Idle --> Searching: QueryChanged(непустой запрос)
    Success --> Searching: QueryChanged(новый непустой запрос)
    Error --> Searching: QueryChanged(новый непустой запрос)

    Searching --> Searching: SearchStarted / SearchProgress [Если query == current_query]
    Searching --> Idle: QueryChanged(пустой запрос) / SearchCancelled
    Searching --> Success: SearchCompleted [query == current_query]
    Searching --> Error: SearchError [query == current_query]

    Success --> Idle: QueryChanged(пустой запрос)
    Success --> Scanning: Смена папки (Change folder)
    Error --> Scanning: Смена папки (Change folder)
```

---

## 6. ERD: Модель данных, кэша и персистентности (Data & Cache Schema)

Поскольку приложение работает без внешней СУБД, роль схемы данных выполняют персистентная конфигурация на диске, индекс файлов в памяти, потокобезопасный кэш текста `TEXT_CACHE` и структуры результатов поиска.

```mermaid
erDiagram
    PERSISTENT_SETTINGS ||--|| APP_STATE : "инициализирует и сохраняет"
    APP_STATE ||--o{ FILE_ENTRY : "содержит проиндексированные (1:N)"
    APP_STATE ||--o{ SEARCH_MATCH : "хранит найденные документы (1:N)"
    SEARCH_MATCH ||--|{ SEARCH_OCCURRENCE : "включает контекстные сниппеты (1:N)"
    FILE_ENTRY ||--o| TEXT_CACHE_ENTRY : "кэшируется по (path, len, mtime) (1:1)"

    PERSISTENT_SETTINGS {
        string config_file_path "Win: %APPDATA%/DocumentSearcher/settings.conf | Linux: ~/.config/DocumentSearcher/settings.conf"
        PathBuf scan_path "Последняя выбранная директория"
    }

    APP_STATE {
        PathBuf current_path "Текущая сканируемая папка"
        string search_query "Активный поисковый запрос"
        enum status "Idle | Scanning | Searching | Success | Error"
        usize results_offset "Текущее смещение пагинации"
        usize results_limit "Лимит на страницу (100)"
        usize skipped_oversized_files "Счётчик файлов > 50 MiB"
        usize skipped_unreadable_entries "Счётчик ошибок чтения"
    }

    FILE_ENTRY {
        PathBuf absolute_path PK "Нормализованный полный путь (ключ домена)"
        string relative_path "Относительный путь с разделителем '/'"
    }

    TEXT_CACHE_ENTRY {
        PathBuf path PK "Часть составного ключа CacheKey"
        u64 file_len PK "Размер файла в байтах"
        SystemTime modified_time PK "Время последней модификации файла"
        Result_ArcString_String cached_value "Извлечённый UTF-8 текст (до 8 MiB) или текст ошибки"
    }

    SEARCH_MATCH {
        PathBuf file_absolute_path PK "Ссылка на FileEntry.absolute_path"
        string file_relative_path "Относительный путь для заголовка карточки"
        Vec_String contexts "Полные строки сниппетов"
    }

    SEARCH_OCCURRENCE {
        string before "До 20 символов UTF-8 перед совпадением"
        string matched "Точная найденная подстрока (для <mark>)"
        string after "До 20 символов UTF-8 после совпадения"
        string snippet "Склеенный фрагмент before + matched + after"
    }
```

---

## 7. Спецификация кроссплатформенного слоя (Windows / ALT Linux / Ubuntu)

| Подсистема | Реализация на Windows (`target_os = "windows"`) | Реализация на ALT Linux и Ubuntu (`target_os = "linux"`) |
| :--- | :--- | :--- |
| **Хранение `settings.conf`** | `%APPDATA%\DocumentSearcher\settings.conf` | `$XDG_CONFIG_HOME/DocumentSearcher/settings.conf` (или `~/.config/DocumentSearcher/settings.conf`) |
| **Диалог выбора папки** | Нативный Win32 COM `SHBrowseForFolderW` (`shell32.dll` / `ole32.dll`) | Системный диалог `zenity --file-selection --directory` с автоматическим откатом на `kdialog --getexistingdirectory` |
| **Открытие документа** | Нативный Win32 `ShellExecuteW(..., "open", ...)` | Системный вызов `xdg-open <путь>` (с откатом на `gio open <путь>`) |
| **Запуск графического окна** | `msedge.exe`, `chrome.exe` или `browser.exe` в режиме `--app=http://127.0.0.1:<port>` (откат на браузер по умолчанию) | `chromium`, `chromium-browser`, `google-chrome`, `msedge` в режиме `--app=http://127.0.0.1:<port>` (откат на `xdg-open`) |
| **Подавление шума `stdout`** | Перенаправление `STD_OUTPUT_HANDLE` в системное устройство `NUL` (`CreateFileW`) | Перенаправление файлового дескриптора `stdout` при необходимости; логирование `tracing` в `stderr` |
| **OCR отсканированных PDF** | `pdftoppm.exe` + `tesseract.exe` (с `CREATE_NO_WINDOW`), откат на встроенный **Windows WinRT (`Windows.Data.Pdf` + `Windows.Media.Ocr`)** | Системные пакеты `/usr/bin/pdftoppm` (`poppler-utils`) + `/usr/bin/tesseract` (`tesseract-ocr`) |
