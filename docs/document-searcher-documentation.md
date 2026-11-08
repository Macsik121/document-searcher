\# Единая техническая спецификация и архитектура системы (SRS \& System Design)



\*\*Проект:\*\* Document Searcher  

\*\*Версия архитектуры:\*\* 2.0 (Unified Cross-Platform Edition)  

\*\*Целевые платформы:\*\* Windows 10/11, ALT Linux (p10/p11), Ubuntu Linux (22.04 / 24.04 LTS)  

\*\*Язык и тулчейн:\*\* Rust Stable (`1.78.0`, Edition 2021)  



\---



\## 1. Назначение и концепция системы



\*\*Document Searcher\*\* — это высокопроизводительное настольное и консольное (CLI + GUI) приложение для глубокого контекстного поиска подстрок внутри локальных документов форматов \*\*`.docx`\*\* и \*\*`.pdf`\*\* (с возможностью расширения списка форматов через параметры запуска).



Приложение спроектировано по принципу \*\*Zero-C-Build-Dependencies\*\*: оно не требует установки тяжёлых компиляторов C/C++ или `\*-devel` пакетов графических тулкитов при сборке из исходного кода и компилируется единой командой `cargo build --release` на Windows, ALT Linux и Ubuntu.



\---



\## 2. Функциональные требования (FR)



\### FR-1: Управление директориями и запуск

\* \*\*FR-1.1:\*\* Приложение поддерживает как графический режим (GUI), так и консольный режим (`scan`, `search`) в едином исполняемом файле `document-searcher`.

\* \*\*FR-1.2:\*\* При запуске без аргументов приоритетно открывается последняя сохранённая папка из конфигурационного файла (`settings.conf`) либо текущая рабочая директория (`current\_dir`).

\* \*\*FR-1.3:\*\* При передаче пути к папке через аргумент командной строки (`document-searcher <путь>` или `document-searcher gui <путь>`) приложение немедленно открывает и индексирует указанную директорию.

\* \*\*FR-1.4:\*\* Пользователь может в любой момент сменить папку через нативный системный диалог выбора директории (Win32 `SHBrowseForFolderW` на Windows; `zenity` / `kdialog` на ALT Linux и Ubuntu). Отмена диалога сохраняет текущий индекс и результаты поиска неизменными.



\### FR-2: Индексация файлов (`scanner.rs`)

\* \*\*FR-2.1:\*\* Рекурсивный обход дерева каталогов выполняется через `WalkDir` без перехода по символическим ссылкам (`follow\_links(false)`) во избежание циклических петель.

\* \*\*FR-2.2:\*\* Скрытые файлы и директории (начинающиеся с `.`) автоматически игнорируются на любом уровне вложенности.

\* \*\*FR-2.3:\*\* По умолчанию индексируются \*\*только файлы `.docx` и `.pdf`\*\* (регистронезависимо). Пользователь может переопределить список расширений через флаг `--ext <список>` или переменную окружения `DOCUMENT\_SEARCHER\_EXTENSIONS`.

\* \*\*FR-2.4:\*\* Файлы размером более \*\*50 MiB\*\* (`52 428 800` байт) пропускаются с увеличением счётчика `oversized\_files`. Нечитаемые записи фиксируются в счётчике `unreadable\_entries`.

\* \*\*FR-2.5:\*\* Все относительные пути нормализуются к единому разделителю `/` и сортируются лексикографически. Расширенные пути Windows (`\\\\?\\C:\\...` и `\\\\?\\UNC\\server\\share\\...`) нормализуются функцией `normalize\_platform\_path`.



\### FR-3: Многоуровневое извлечение текста и кэширование (`extractor.rs`)

\* \*\*FR-3.1 (In-Memory Кэш `TEXT\_CACHE`):\*\* Извлечённый текст каждого файла кэшируется в потокобезопасном хранилище `OnceLock<RwLock<HashMap<CacheKey, CacheValue>>>` по составному ключу `(PathBuf, размер\_файла, время\_модификации)`. Повторные поисковые запросы читают текст из ОЗУ за микросекунды без повторного обращения к диску. Ошибки повреждённых файлов также кэшируются, чтобы не парсить сбойные файлы повторно.

\* \*\*FR-3.2 (Извлечение `.docx`):\*\* Выполняется путём потокового чтения `word/document.xml` из ZIP-архива через `quick-xml` с сохранением абзацев (`w:p`), переносов (`w:br`) и табуляций (`w:tab`).

\* \*\*FR-3.3 (Трёхуровневый конвейер `.pdf`):\*\*

&#x20; 1. \*\*Уровень 1 — Постраничный декодер `lopdf` с поддержкой кириллицы:\*\* Разбирает PDF постранично, декодируя таблицы шрифтов `/Encoding -> /Differences` (включая глифы `/uni0410`..`/uni044F`, `/afii10017`..`/afii10097`, `.cap`-глифы), потоки `/ToUnicode` (`beginbfchar` и `beginbfrange`), `UTF-16BE` и однобайтовую кириллицу `CP1251`.

&#x20; 2. \*\*Уровень 2 — Изолированный `pdf-extract`:\*\* Запускается внутри `std::panic::catch\_unwind` (защита от паник на сложных цветовых функциях PDF, таких как \*Function Type 4\*). Выбирается результат с наибольшим числом буквенных символов.

&#x20; 3. \*\*Уровень 3 — OCR для отсканированных PDF без текстового слоя:\*\* Если текстовый слой пуст, вызывается рендеринг страниц через `pdftoppm` + `tesseract` (`eng+rus` с автоматическим откатом на `eng`), а на Windows при отсутствии внешних утилит — встроенный системный движок \*\*Windows WinRT (`Windows.Data.Pdf` + `Windows.Media.Ocr`)\*\*.



\### FR-4: Регистронезависимый поиск и многопоточность (`search.rs`, `worker.rs`)

\* \*\*FR-4.1 (Unicode Case-Folding):\*\* Поиск сопоставляет подстроку без учёта регистра через структуру `FoldedText`, сохраняющую точные байтовые границы (`source\_spans`) каждого символа исходного текста UTF-8.

\* \*\*FR-4.2 (Контекстные сниппеты):\*\* Для каждого вхождения формируется структура `SearchOccurrence` с контекстом \*\*до 20 символов до совпадения\*\* (`before`), \*\*точной найденной подстрокой\*\* (`matched`) и \*\*до 20 символов после совпадения\*\* (`after`).

\* \*\*FR-4.3 (Debounce и отмена):\*\* Поиск запускается с задержкой \*\*300 мс\*\* после последнего ввода символа. Новый ввод немедленно отменяет текущий поиск через `CancellationToken` и `join\_set.abort\_all()`.

\* \*\*FR-4.4 (Параллельный пул `JoinSet` + `Semaphore`):\*\* Извлечение и поиск выполняются параллельно на всех доступных ядрах CPU (`available\_parallelism()`, от 2 до 8 потоков одновременно) с детерминированной итоговой сортировкой результатов.



\### FR-5: Графический интерфейс, пагинация и безопасность (`app.rs` / `app\_win.rs`)

\* \*\*FR-5.1 (Разделение экрана 50/50):\*\* Левая панель отображает текущий путь, количество найденных файлов, прокручиваемый список файлов и кнопку смены папки. Правая панель содержит поле поиска, индикатор состояния (спиннер / зелёная галочка / ошибка) и карточки результатов с жёлтым выделением совпадений (`<mark>`).

\* \*\*FR-5.2 (Кумулятивная пагинация):\*\* Результаты выдаются порциями по \*\*100 документов\*\*. Кнопка `"Show more"` догружает следующие 100 результатов в список без удаления предыдущих.

\* \*\*FR-5.3 (Безопасное открытие файлов):\*\* Клик по файлу открывает его в программе ОС по умолчанию (Win32 `ShellExecuteW` на Windows; `xdg-open` на ALT Linux / Ubuntu) только после проверки принадлежности пути к белому списку текущего индекса (`is\_indexed\_file`) и проверки сессионного CSRF-токена (`X-Document-Searcher-Token`).



\---



\## 3. Нефункциональные требования (NFR)



\* \*\*NFR-1 (Отзывчивость UI):\*\* Поток управления состоянием UI никогда не блокируется файловым вводом-выводом или парсингом PDF.

\* \*\*NFR-2 (Отказоустойчивость):\*\* Паника парсера, повреждённый ZIP/PDF или ошибка чтения одного файла перехватываются на уровне файла (`catch\_unwind` + `Result`), логируются через `tracing::warn!` и никогда не прерывают пакетный поиск по остальным файлам.

\* \*\*NFR-3 (Автономность):\*\* Вся вёрстка, стили и скрипты встроены в исполняемый файл (`INDEX\_HTML`) и работают на 100% офлайн без внешних CDN.



\---



\## 4. BPMN 2.0: Сквозной бизнес-процесс работы системы



Диаграмма ниже описывает полный цикл работы приложения в разрезе трёх дорожек (Swimlanes): \*\*Пользователь\*\*, \*\*UI \& Synchronized Controller\*\* и \*\*Async Tokio Worker \& Extraction Pool\*\*.



```mermaid

flowchart TB

&#x20;   subgraph Lane\_User\["👤 Дорожка 1: Пользователь"]

&#x20;       U\_Start(\["Запуск приложения\\n(CLI или GUI)"])

&#x20;       U\_ChangeFolder\["Нажатие 'Change folder'\\nВыбор папки в диалоге ОС"]

&#x20;       U\_TypeQuery\["Ввод поискового запроса\\nв строку поиска"]

&#x20;       U\_ClickMore\["Нажатие 'Show more'\\n(Пагинация +100)"]

&#x20;       U\_ClickFile\["Клик по файлу\\n(Открытие документа)"]

&#x20;   end



&#x20;   subgraph Lane\_Controller\["🖥️ Дорожка 2: UI, HttpWorkerPool (4 потока) \& Монитор Controller"]

&#x20;       C\_Init\["Инициализация Controller\\nЧтение settings.conf / CLI аргумента\\nГенерация Session Token"]

&#x20;       C\_Dialog\["Открытие системного диалога папки\\n(Win32 SHBrowseForFolderW / Linux zenity)"]

&#x20;       C\_DialogCheck{"Папка\\nвыбрана?"}

&#x20;       C\_SavePath\["Сохранение пути в settings.conf\\nАтомарный сброс состояния и отправка Scan"]

&#x20;       C\_QueryGate{"Проверка X-Query-Seq\\nseq >= last\_query\_seq?"}

&#x20;       C\_AtomicSubmit\["Атомарно под Mutex:\\nОбновление search\_query\\nОтправка QueryChanged в Worker"]

&#x20;       C\_RenderUI\["Отрисовка состояния в UI:\\nСпиннер / Прогресс / Карточки с <mark>"]

&#x20;       C\_LoadMore\["Вызов state.load\_more()\\nРасширение среза +100"]

&#x20;       C\_SecurityCheck{"Токен валиден И\\nфайл в found\_files?"}

&#x20;       C\_OpenOS\["Открытие в ОС по умолчанию\\n(Win32 ShellExecuteW / Linux xdg-open)"]

&#x20;   end



&#x20;   subgraph Lane\_Worker\["⚙️ Дорожка 3: Tokio Worker, JoinSet Pool \& 3-Tier Extractor"]

&#x20;       W\_Scan\["Обход WalkDir (только .docx и .pdf)\\nЛимит 50 MiB, пропуск скрытых и симлинков"]

&#x20;       W\_ScanDone\["Сортировка путей\\nОтправка WorkerEvent::ScanCompleted"]

&#x20;       W\_CancelOld\["Отмена предыдущего поиска:\\nCancellationToken::cancel()\\njoin\_set.abort\_all()"]

&#x20;       W\_Debounce{"Ожидание Debounce\\n300 мс (или отмена)"}

&#x20;       W\_Parallel\["Запуск параллельного пула JoinSet\\nОграничение: Semaphore(CPU cores 2..8)"]

&#x20;       W\_CacheCheck{"Файл есть в\\nTEXT\_CACHE?"}

&#x20;       W\_Extract\["3-уровневое извлечение:\\n1. DOCX XML / lopdf (кириллица uni04XX, ToUnicode, CP1251)\\n2. pdf-extract (внутри catch\_unwind)\\n3. OCR Fallback (Tesseract / WinRT)"]

&#x20;       W\_SaveCache\["Запись результата/ошибки\\nв TEXT\_CACHE"]

&#x20;       W\_Match\["Unicode Case-Folded поиск (FoldedText)\\nФормирование контекста ±20 символов"]

&#x20;       W\_Complete\["Детерминированная сортировка\\nОтправка SearchCompleted"]

&#x20;   end



&#x20;   U\_Start --> C\_Init --> W\_Scan --> W\_ScanDone --> C\_RenderUI

&#x20;   U\_ChangeFolder --> C\_Dialog --> C\_DialogCheck

&#x20;   C\_DialogCheck -- "Отмена" --> C\_RenderUI

&#x20;   C\_DialogCheck -- "Выбрана" --> C\_SavePath --> W\_Scan



&#x20;   U\_TypeQuery --> C\_QueryGate

&#x20;   C\_QueryGate -- "Устаревший пакет (отбросить)" --> C\_RenderUI

&#x20;   C\_QueryGate -- "Актуальный" --> C\_AtomicSubmit --> W\_CancelOld --> W\_Debounce

&#x20;   W\_Debounce -- "Новый ввод до 300 мс" --> W\_CancelOld

&#x20;   W\_Debounce -- "300 мс истекли" --> W\_Parallel --> W\_CacheCheck

&#x20;   W\_CacheCheck -- "Cache Hit (ОЗУ)" --> W\_Match

&#x20;   W\_CacheCheck -- "Cache Miss (Диск)" --> W\_Extract --> W\_SaveCache --> W\_Match

&#x20;   W\_Match --> W\_Complete --> C\_RenderUI



&#x20;   U\_ClickMore --> C\_LoadMore --> C\_RenderUI

&#x20;   U\_ClickFile --> C\_SecurityCheck

&#x20;   C\_SecurityCheck -- "Да" --> C\_OpenOS

&#x20;   C\_SecurityCheck -- "Нет (403 Forbidden)" --> C\_RenderUI

```



\---



\## 5. UML-диаграммы архитектуры



\### 5.1. UML Component Diagram (Диаграмма компонентов и платформенных адаптеров)



```mermaid

flowchart TB

&#x20;   subgraph Presentation\["Слой представления и маршрутизации"]

&#x20;       Main\["src/main.rs\\nУниверсальная точка входа (GUI + CLI)"]

&#x20;       WebUI\["Embedded Offline UI (INDEX\_HTML)\\nМакет app-design.html + AbortController"]

&#x20;       HttpPool\["HttpWorkerPool\\n4 постоянных потока + sync\_channel(32)"]

&#x20;   end



&#x20;   subgraph Synchronization\["Слой синхронизации (Monitor Boundary)"]

&#x20;       Ctrl\["Controller (Arc<Mutex<Controller>>)\\nЕдиная точка сериализации стейта и команд"]

&#x20;       UiState\["UiState\\nКонечный автомат UI + проверка белого списка"]

&#x20;       Bridge\["RuntimeBridge\\nУправляемый жизненный цикл потока Tokio + Drop Shutdown"]

&#x20;   end



&#x20;   subgraph AsyncCore\["Асинхронное ядро (Feature-Free Domain Core)"]

&#x20;       Worker\["model/worker.rs\\nTokio Worker (Debounce 300ms, CancellationToken, JoinSet)"]

&#x20;       Scanner\["model/scanner.rs\\nWalkDir (.docx, .pdf, <=50 MiB)"]

&#x20;       Search\["model/search.rs\\nFoldedText Unicode Matcher (±20 символов)"]

&#x20;       Extractor\["model/extractor.rs\\nTEXT\_CACHE + lopdf Кириллица + pdf-extract + OCR"]

&#x20;       State\["model/state.rs \& actions.rs\\nAppState, WorkerCommand, WorkerEvent"]

&#x20;   end



&#x20;   subgraph PlatformAdapter\["Кроссплатформенный адаптер ОС (platform)"]

&#x20;       WinAdapter\["#\[cfg(target\_os = 'windows')]\\nSHBrowseForFolderW, ShellExecuteW,\\nWinRT OCR, Tesseract/Poppler CLI"]

&#x20;       LinuxAdapter\["#\[cfg(target\_os = 'linux')]\\nALT Linux / Ubuntu:\\nzenity / kdialog, xdg-open,\\nTesseract + pdftoppm CLI"]

&#x20;   end



&#x20;   Main --> HttpPool

&#x20;   Main --> Scanner

&#x20;   Main --> Search

&#x20;   WebUI <-->|"HTTP JSON / X-Document-Searcher-Token"| HttpPool

&#x20;   HttpPool --> Ctrl

&#x20;   Ctrl --> UiState

&#x20;   Ctrl --> Bridge

&#x20;   Bridge <-->|"mpsc::channel\\nWorkerCommand / WorkerEvent"| Worker

&#x20;   Worker --> Scanner

&#x20;   Worker --> Search

&#x20;   Search --> Extractor

&#x20;   UiState --> State

&#x20;   Worker --> State

&#x20;   Ctrl --> PlatformAdapter

&#x20;   Extractor --> PlatformAdapter

&#x20;   PlatformAdapter --> WinAdapter

&#x20;   PlatformAdapter --> LinuxAdapter

```



\---



\### 5.2. UML Class Diagram (Диаграмма классов и структур данных Rust)



```mermaid

classDiagram

&#x20;   class Controller {

&#x20;       -ui: UiState

&#x20;       -bridge: RuntimeBridge

&#x20;       +submit\_query(query: String, seq: Option\~u64\~)

&#x20;       +start\_folder\_scan(folder: PathBuf)

&#x20;   }



&#x20;   class UiState {

&#x20;       +state: AppState

&#x20;       +status\_text: String

&#x20;       +version: u64

&#x20;       +last\_query\_seq: u64

&#x20;       +folder\_dialog\_open: bool

&#x20;       +is\_indexed\_file(candidate: \&Path) bool

&#x20;       +accept\_query\_edit(query: String, seq: Option\~u64\~) bool

&#x20;       +scan\_started()

&#x20;       +scan\_completed(files: Vec\~FileEntry\~, oversized: usize, unreadable: usize)

&#x20;       +search\_started(query: \&str)

&#x20;       +search\_progress(completed: usize, total: usize)

&#x20;       +search\_completed(query: String, results: Vec\~SearchMatch\~)

&#x20;       +load\_more()

&#x20;   }



&#x20;   class RuntimeBridge {

&#x20;       -commands: mpsc::Sender\~WorkerCommand\~

&#x20;       -runtime\_thread: Option\~JoinHandle\~

&#x20;       +start() (RuntimeBridge, mpsc::Receiver\~WorkerEvent\~)

&#x20;       +send(command: WorkerCommand)

&#x20;   }



&#x20;   class Worker {

&#x20;       -files: Arc\~RwLock\~Vec\~FileEntry\~\~\~\~

&#x20;       -search\_task: Option\~JoinHandle\~

&#x20;       -search\_cancel: Option\~CancellationToken\~

&#x20;       -scan\_task: Option\~JoinHandle\~

&#x20;       -search\_generation: Arc\~AtomicU64\~

&#x20;       -scan\_generation: Arc\~AtomicU64\~

&#x20;       -extraction\_slots: Arc\~Semaphore\~

&#x20;       +run(commands: Receiver\~WorkerCommand\~, events: Sender\~WorkerEvent\~)

&#x20;   }



&#x20;   class AppState {

&#x20;       +settings: AppSettings

&#x20;       +current\_path: PathBuf

&#x20;       +found\_files: Vec\~FileEntry\~

&#x20;       +search\_query: String

&#x20;       +search\_results: Vec\~SearchMatch\~

&#x20;       +results\_offset: usize

&#x20;       +results\_limit: usize

&#x20;       +status: SearchStatus

&#x20;       +error: Option\~String\~

&#x20;       +skipped\_oversized\_files: usize

&#x20;       +skipped\_unreadable\_entries: usize

&#x20;       +set\_scan\_path(path: PathBuf)

&#x20;       +get\_visible\_results() \&\[SearchMatch]

&#x20;       +has\_more\_results() bool

&#x20;       +load\_more()

&#x20;   }



&#x20;   class FileEntry {

&#x20;       +relative\_path: String

&#x20;       +absolute\_path: PathBuf

&#x20;   }



&#x20;   class SearchMatch {

&#x20;       +file\_relative\_path: String

&#x20;       +file\_absolute\_path: PathBuf

&#x20;       +contexts: Vec\~String\~

&#x20;       +occurrences: Vec\~SearchOccurrence\~

&#x20;   }



&#x20;   class SearchOccurrence {

&#x20;       +before: String

&#x20;       +matched: String

&#x20;       +after: String

&#x20;       +snippet: String

&#x20;   }



&#x20;   class CacheKey {

&#x20;       +path: PathBuf

&#x20;       +len: u64

&#x20;       +modified: Option\~SystemTime\~

&#x20;   }



&#x20;   class FontCharMap {

&#x20;       +single\_byte: HashMap\~u8, String\~

&#x20;       +two\_byte: HashMap\~u16, String\~

&#x20;   }



&#x20;   Controller \*-- UiState

&#x20;   Controller \*-- RuntimeBridge

&#x20;   UiState \*-- AppState

&#x20;   AppState "1" \*-- "\*" FileEntry

&#x20;   AppState "1" \*-- "\*" SearchMatch

&#x20;   SearchMatch "1" \*-- "\*" SearchOccurrence

&#x20;   RuntimeBridge ..> Worker : commands / events

```



\---



\### 5.3. UML Sequence Diagram (Многопоточный поиск, отмена и работа кэша)



```mermaid

sequenceDiagram

&#x20;   autonumber

&#x20;   actor User as Пользователь

&#x20;   participant UI as Окно UI (JS + AbortController)

&#x20;   participant Pool as HttpWorkerPool (4 потока)

&#x20;   participant Ctrl as Controller (Mutex)

&#x20;   participant W as Tokio Worker

&#x20;   participant JS as JoinSet + Semaphore(N ядер)

&#x20;   participant Cache as TEXT\_CACHE (RwLock)

&#x20;   participant Ext as 3-Tier Extractor (lopdf / docx / OCR)



&#x20;   User->>UI: Вводит "химия" (символ за символом)

&#x20;   UI->>UI: AbortController.abort() старого запроса\\nquerySeq += 1

&#x20;   UI->>Pool: POST /api/query (X-Query-Seq: 10, body: "химия")

&#x20;   Pool->>Ctrl: lock() -> submit\_query("химия", Some(10))

&#x20;   Ctrl->>Ctrl: Проверка 10 >= last\_query\_seq\\nstatus = Searching ("Waiting for input…")

&#x20;   Ctrl->>W: WorkerCommand::QueryChanged { query: "химия" }

&#x20;   Ctrl-->>Pool: unlock()

&#x20;   Pool-->>UI: 200 OK



&#x20;   W->>W: cancel\_search() (CancellationToken::cancel)\\ngeneration += 1

&#x20;   W->>W: Ожидание SEARCH\_DEBOUNCE (300 мс)

&#x20;   W-->>Ctrl: WorkerEvent::SearchStarted { query: "химия" }



&#x20;   loop Параллельно для всех файлов (JoinSet до 8 потоков)

&#x20;       W->>JS: spawn(file, query, semaphore\_permit)

&#x20;       JS->>Cache: read().get(CacheKey { path, len, mtime })

&#x20;       alt Cache Hit (Уже в памяти)

&#x20;           Cache-->>JS: Ok(Arc<String>) за \~0.001 мс

&#x20;       else Cache Miss (Чтение с диска)

&#x20;           JS->>Ext: extract\_text\_uncached(path)

&#x20;           Ext-->>JS: Декодированный UTF-8 текст (lopdf / docx / OCR)

&#x20;           JS->>Cache: write().insert(CacheKey, Ok(Arc<String>))

&#x20;       end

&#x20;       JS->>JS: FoldedText::new() -> поиск вхождений ±20 символов

&#x20;       JS-->>W: Vec<SearchMatch>

&#x20;       W-->>Ctrl: WorkerEvent::SearchProgress { completed, total }

&#x20;   end



&#x20;   W->>W: Сортировка результатов по relative\_path

&#x20;   W-->>Ctrl: WorkerEvent::SearchCompleted { query: "химия", results }

&#x20;   UI->>Pool: GET /api/state

&#x20;   Pool-->>UI: JSON (status: "success", results с подсветкой)

```



\---



\### 5.4. UML State Machine Diagram (Конечный автомат состояний `SearchStatus`)



```mermaid

stateDiagram-v2

&#x20;   \[\*] --> Idle: Инициализация AppState



&#x20;   Idle --> Scanning: WorkerCommand::Scan / ScanStarted

&#x20;   Scanning --> Idle: ScanCompleted (N файлов проиндексировано)

&#x20;   Scanning --> Error: ScanError (Папка недоступна)



&#x20;   Scanning --> Scanning: QueryChanged(пустой) \[Игнорируется, не сбивает Scanning]



&#x20;   Idle --> Searching: QueryChanged(непустой запрос)

&#x20;   Success --> Searching: QueryChanged(новый непустой запрос)

&#x20;   Error --> Searching: QueryChanged(новый непустой запрос)



&#x20;   Searching --> Searching: SearchStarted / SearchProgress \[Если query == current\_query]

&#x20;   Searching --> Idle: QueryChanged(пустой запрос) / SearchCancelled

&#x20;   Searching --> Success: SearchCompleted \[query == current\_query]

&#x20;   Searching --> Error: SearchError \[query == current\_query]



&#x20;   Success --> Idle: QueryChanged(пустой запрос)

&#x20;   Success --> Scanning: Смена папки (Change folder)

&#x20;   Error --> Scanning: Смена папки (Change folder)

```



\---



\## 6. ERD: Модель данных, кэша и персистентности (Data \& Cache Schema)



Поскольку приложение работает без внешней СУБД, роль схемы данных выполняют персистентная конфигурация на диске, индекс файлов в памяти, потокобезопасный кэш текста `TEXT\_CACHE` и структуры результатов поиска.



```mermaid

erDiagram

&#x20;   PERSISTENT\_SETTINGS ||--|| APP\_STATE : "инициализирует и сохраняет"

&#x20;   APP\_STATE ||--o{ FILE\_ENTRY : "содержит проиндексированные (1:N)"

&#x20;   APP\_STATE ||--o{ SEARCH\_MATCH : "хранит найденные документы (1:N)"

&#x20;   SEARCH\_MATCH ||--|{ SEARCH\_OCCURRENCE : "включает контекстные сниппеты (1:N)"

&#x20;   FILE\_ENTRY ||--o| TEXT\_CACHE\_ENTRY : "кэшируется по (path, len, mtime) (1:1)"



&#x20;   PERSISTENT\_SETTINGS {

&#x20;       string config\_file\_path "Win: %APPDATA%/DocumentSearcher/settings.conf | Linux: \~/.config/DocumentSearcher/settings.conf"

&#x20;       PathBuf scan\_path "Последняя выбранная директория"

&#x20;   }



&#x20;   APP\_STATE {

&#x20;       PathBuf current\_path "Текущая сканируемая папка"

&#x20;       string search\_query "Активный поисковый запрос"

&#x20;       enum status "Idle | Scanning | Searching | Success | Error"

&#x20;       usize results\_offset "Текущее смещение пагинации"

&#x20;       usize results\_limit "Лимит на страницу (100)"

&#x20;       usize skipped\_oversized\_files "Счётчик файлов > 50 MiB"

&#x20;       usize skipped\_unreadable\_entries "Счётчик ошибок чтения"

&#x20;   }



&#x20;   FILE\_ENTRY {

&#x20;       PathBuf absolute\_path PK "Нормализованный полный путь (ключ домена)"

&#x20;       string relative\_path "Относительный путь с разделителем '/'"

&#x20;   }



&#x20;   TEXT\_CACHE\_ENTRY {

&#x20;       PathBuf path PK "Часть составного ключа CacheKey"

&#x20;       u64 file\_len PK "Размер файла в байтах"

&#x20;       SystemTime modified\_time PK "Время последней модификации файла"

&#x20;       Result\_ArcString\_String cached\_value "Извлечённый UTF-8 текст (до 8 MiB) или текст ошибки"

&#x20;   }



&#x20;   SEARCH\_MATCH {

&#x20;       PathBuf file\_absolute\_path PK "Ссылка на FileEntry.absolute\_path"

&#x20;       string file\_relative\_path "Относительный путь для заголовка карточки"

&#x20;       Vec\_String contexts "Полные строки сниппетов"

&#x20;   }



&#x20;   SEARCH\_OCCURRENCE {

&#x20;       string before "До 20 символов UTF-8 перед совпадением"

&#x20;       string matched "Точная найденная подстрока (для <mark>)"

&#x20;       string after "До 20 символов UTF-8 после совпадения"

&#x20;       string snippet "Склеенный фрагмент before + matched + after"

&#x20;   }

```



\---



\## 7. Спецификация кроссплатформенного слоя (Windows / ALT Linux / Ubuntu)



| Подсистема | Реализация на Windows (`target\_os = "windows"`) | Реализация на ALT Linux и Ubuntu (`target\_os = "linux"`) |

| :--- | :--- | :--- |

| \*\*Хранение `settings.conf`\*\* | `%APPDATA%\\DocumentSearcher\\settings.conf` | `$XDG\_CONFIG\_HOME/DocumentSearcher/settings.conf` (или `\~/.config/DocumentSearcher/settings.conf`) |

| \*\*Диалог выбора папки\*\* | Нативный Win32 COM `SHBrowseForFolderW` (`shell32.dll` / `ole32.dll`) | Системный диалог `zenity --file-selection --directory` с автоматическим откатом на `kdialog --getexistingdirectory` |

| \*\*Открытие документа\*\* | Нативный Win32 `ShellExecuteW(..., "open", ...)` | Системный вызов `xdg-open <путь>` (с откатом на `gio open <путь>`) |

| \*\*Запуск графического окна\*\* | `msedge.exe`, `chrome.exe` или `browser.exe` в режиме `--app=http://127.0.0.1:<port>` (откат на браузер по умолчанию) | `chromium`, `chromium-browser`, `google-chrome`, `msedge` в режиме `--app=http://127.0.0.1:<port>` (откат на `xdg-open`) |

| \*\*Подавление шума `stdout`\*\* | Перенаправление `STD\_OUTPUT\_HANDLE` в системное устройство `NUL` (`CreateFileW`) | Перенаправление файлового дескриптора `stdout` при необходимости; логирование `tracing` в `stderr` |

| \*\*OCR отсканированных PDF\*\* | `pdftoppm.exe` + `tesseract.exe` (с `CREATE\_NO\_WINDOW`), откат на встроенный \*\*Windows WinRT (`Windows.Data.Pdf` + `Windows.Media.Ocr`)\*\* | Системные пакеты `/usr/bin/pdftoppm` (`poppler-utils`) + `/usr/bin/tesseract` (`tesseract-ocr`) |



\## 8. Аппаратные требования и бюджет системных ресурсов (Hardware Constraints \& Resource Budget)



Архитектура приложения спроектирована с жёсткими верхними границами потребления ресурсов, чтобы система стабильно работала как на современных рабочих станциях, так и внутри ограниченных виртуальных машин (например, \*\*ALT Linux VM\*\* в VirtualBox / QEMU / VMware).



\### 8.1. Сводная таблица аппаратных требований



| Параметр оборудования | Минимальные требования (CLI / Малые папки до 200 файлов) | Минимальные требования (GUI / Виртуальная машина ALT Linux) | Рекомендуемые характеристики (Базы > 2000 PDF/DOCX + активный OCR) |

| :--- | :--- | :--- | :--- |

| \*\*Архитектура CPU\*\* | `x86\_64` (`amd64`) или `aarch64` (`ARM64`) | `x86\_64` (`amd64`) | `x86\_64` / `aarch64` |

| \*\*Количество ядер CPU\*\* | 1–2 логических ядра | \*\*2 логических ядра (vCPU)\*\* | \*\*4–8+ логических ядер\*\* |

| \*\*Оперативная память (RAM)\*\* | \*\*512 МБ\*\* (только CLI) | \*\*2 ГБ ОЗУ\*\* | \*\*4–8 ГБ ОЗУ\*\* |

| \*\*Свободное место на диске (Runtime)\*\* | \*\*50 МБ\*\* | \*\*500 МБ\*\* (с учётом буфера `%TEMP%` / `/tmp` для OCR) | \*\*1 ГБ+\*\* на SSD |

| \*\*Свободное место для сборки (`cargo build`)\*\* | \*\*2.5 ГБ\*\* (папка `target/` и кэш Cargo) | \*\*2.5 ГБ\*\* | \*\*4 ГБ+\*\* |

| \*\*Тип накопителя\*\* | HDD / виртуальный диск VM | HDD / виртуальный диск VM | \*\*SSD / NVMe\*\* (ускоряет первичный холодный парсинг PDF в 4–6 раз) |

| \*\*Видеоядро и дисплей\*\* | Не требуется (Headless / SSH) | Любой видеоадаптер VM без 3D-ускорения, разрешение от `1024×768` | Разрешение от `1280×800` (окно по умолчанию `1200×800`) |



\---



\### 8.2. Инженерный расчёт потребления ресурсов и защитные лимиты в коде



Каждое аппаратное ограничение защищено соответствующей константой в исходном коде Rust:



\#### 1. Бюджет процессорного времени (CPU \& Threading)

\* \*\*Фоновый пул извлечения (`worker.rs`):\*\*

&#x20; Количество параллельных задач `spawn\_blocking` определяется формулой:

&#x20; $$\\text{Slots} = \\text{clamp}(\\text{available\\\_parallelism}(), 2, 8)$$

&#x20; \* На слабых двухъядерных машинах или в VM с 1–2 vCPU пул ограничивается \*\*2 потоками\*\*, не вызывая голодания потока интерфейса.

&#x20; \* На многоядерных процессорах (8, 12, 16+ ядер) пул намеренно ограничен сверху \*\*8 потоками\*\*, чтобы параллельная распаковка тяжёлых PDF не забивала контроллер памяти и кэш L3 процессора.

\* \*\*Пул HTTP/IPC сервера (`app\_win.rs`):\*\*

&#x20; Фиксирован ровно на \*\*4 спящих потоках\*\* (`HTTP\_WORKER\_THREADS = 4`) с блокирующим ожиданием на канале `sync\_channel(32)` (потребление CPU в простое — \*\*0.0%\*\*).

\* \*\*Защита от перегрузки при вводе (`SEARCH\_DEBOUNCE = 300ms` + `CancellationToken`):\*\*

&#x20; Предотвращает запуск конвейера парсинга до окончания набора слова пользователем и мгновенно освобождает ядра CPU при изменении запроса (`join\_set.abort\_all()`).



\#### 2. Бюджет оперативной памяти (RAM \& Защита от Out-Of-Memory)

\* \*\*Ограничение размера одного файла (`scanner.rs` → `MAX\_FILE\_SIZE = 50 MiB`):\*\*

&#x20; Любой файл $> 50\\text{ MiB}$ отсекается на этапе чтения метаданных `fs::metadata` до открытия файла. Это гарантирует, что ни один повреждённый или гигантский архив не вызовет резкого скачка аллокации в куче.

\* \*\*Ограничение In-Memory кэша текста (`extractor.rs` → `TEXT\_CACHE`):\*\*

&#x20; \* Максимальный размер кэшируемого текста одного документа: `MAX\_CACHED\_TEXT\_BYTES = 8 MiB`. Документы с объёмом извлечённого чистого текста $> 8\\text{ MiB}$ обрабатываются на лету без сохранения в кэш.

&#x20; \* Максимальное число документов в кэше: `MAX\_CACHE\_ENTRIES = 4096`. При достижении 4096 записей кэш автоматически очищается (`guard.clear()`).

&#x20; \* \*\*Типичное потребление ОЗУ кэшем:\*\* Для базы из \*\*2100 документов\*\* (средний объём извлечённого текста \~80 КБ на документ) весь кэш `TEXT\_CACHE` занимает в оперативной памяти всего \*\*\~160–200 МБ\*\*, обеспечивая мгновенный повторный поиск.

\* \*\*Ограничение сетевого буфера IPC (`MAX\_REQUEST\_BYTES = 64 KiB`):\*\*

&#x20; Защищает локальный сервер от некорректных или переполненных пакетов.



\#### 3. Бюджет дискового ввода-вывода (Disk I/O \& Временные файлы OCR)

\* \*\*Холодный vs. Тёплый проход:\*\*

&#x20; Интенсивное чтение с диска происходит только при \*\*первом\*\* поисковом запросе в сессии (Cache Miss). Все последующие запросы в рамках той же папки работают исключительно с ОЗУ (0 операций чтения содержимого файлов с диска).

\* \*\*Временная директория для OCR (`extract\_scanned\_pdf\_text`):\*\*

&#x20; \* При распознавании отсканированных PDF страницы рендерятся во временную папку `%TEMP%\\document-searcher-pdf-<PID>-<TIMESTAMP>` (или `/tmp/...` на Linux) с разрешением `200 DPI` (при использовании Windows WinRT OCR — рендеринг происходит в памяти через `InMemoryRandomAccessStream` без записи на диск и ограничен первыми 15 страницами документа).

&#x20; \* Временная папка гарантированно удаляется (`fs::remove\_dir\_all`) в блоке финализации сразу после завершения обработки каждого PDF-файла, не оставляя мусора на диске.

