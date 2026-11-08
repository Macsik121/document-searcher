# Устаревший план разработки Document Searcher

> Исторический документ: описанные ниже Relm4/Blueprint шаги и старые этапы
> реализации не являются архитектурой текущего runnable приложения. Для
> текущего состояния используйте `docs/architecture/` и
> `docs/ALT-LINUX-RUN.md`.

## Context

Репозиторий содержит каркас Rust/GTK4-приложения и набор спецификаций в `docs/specs`, но рабочей системы пока нет: `main.rs` — заглушка, `app.rs` вызывает отсутствующий Tokio runtime и несогласованные сообщения, виджеты не подключены к состоянию, а части кода не соответствуют зависимостям (`tesseract_rs`/`leptess`, `ashpd` и `serde` отсутствуют в `Cargo.toml`, `gtk`/`gtk4` используются неодинаково). Старый `src/document-searcher-implementation-plan.txt` полезен как источник намерений, но содержит устаревшие форматы, сомнительные API и незавершённые шаги.

Цель — довести MVP до компилируемой и проверяемой системы для ALT Linux/Flatpak: выбор и сохранение каталога, безопасное сканирование, нативное извлечение текста и OCR только там, где это нужно, отменяемый debounce-поиск, результаты с контекстом и открытие файлов. Предсуществующие изменения переноса спецификаций из `src/specs` в `docs/specs` не трогать.

## Рекомендуемый подход

### 1. Стабилизировать контракт сборки и структуру приложения

- Привести `src/Cargo.toml` к согласованным версиям GTK4/Libadwaita/Relm4 и добавить реально используемые `ashpd`, `serde`, `serde_json`/`tracing` при необходимости; заменить неправильные импорты и выбрать `leptess` как единственную Rust-обёртку Tesseract.
- Исправить `main.rs`: настроить tracing, создать и корректно завершать выделенный Tokio runtime/worker, затем запускать Relm4/GTK main loop. GTK-код не должен вызывать `tokio::spawn` без runtime.
- Историческое намерение этого этапа — устранить дублирование UI-источников. Текущая реализация использует procedural UI в `app.rs`; Blueprint и прежние widgets не входят в runnable pipeline.
- Унифицировать идентификатор приложения `org.gtk.DocumentSearcher` во всех ресурсах, desktop entry, metainfo, GResource и Flatpak manifest.

Критические файлы: `src/Cargo.toml`, `src/src/main.rs`, `src/src/app.rs`, `src/build.rs`, `src/data/resources/window.blp`, `src/data/resources/gresource.xml`, `src/data/org.gtk.DocumentSearcher.desktop`, `src/data/org.gtk.DocumentSearcher.metainfo.xml`.

### 2. Реализовать доменную модель и безопасный scanner

- Пересобрать `model/state.rs` вокруг детерминированного ключа `PathBuf`: настройки каталога, `HashMap`/список найденных документов, query, статус (`Idle/Scanning/Searching/Success/Error`), результаты и pagination.
- В `model/scanner.rs` поддержать полный список из SRS: текстовые форматы, PDF, DOCX и изображения; расширение сравнивать без учёта регистра.
- Использовать `WalkDir` без следования symlink, пропускать скрытые файлы и каталоги на каждом уровне, обрабатывать ошибки обхода без падения всего сканирования и пропускать файлы больше 50 MiB. Возвращать отсортированные относительный и абсолютный пути и статистику пропусков.
- Добавить unit-тесты для форматов, hidden/symlink-политики, лимита размера, сортировки и ошибок доступа (с временной директорией).

Критические файлы: `src/src/model/state.rs`, `src/src/model/scanner.rs`, `src/src/model/actions.rs`; новые тестовые fixtures — в `src/tests` или inline tempfile-тестах.

### 3. Реализовать composite extraction без ложного OCR

- Текстовые расширения (`txt`, `md`, `rs`, `toml`, `json`, `xml`, `html`, `css`, `js`, `ts`) читать потоково/с ограниченным размером и корректно обрабатывать invalid UTF-8.
- DOCX извлекать из XML/`docx-rs`; PDF сначала обрабатывать через native text layer (`pdf-extract`/`lopdf`), а OCR запускать только если текстовый слой пуст.
- Для `png/jpg/jpeg/tiff/bmp` использовать `leptess`; не передавать DOCX/PDF/RTF/ODT напрямую в Tesseract. Для неподдержанного или повреждённого файла возвращать ошибку на уровне файла, логировать её и продолжать поиск.
- Вынести настройки tessdata/languages из кода через окружение Flatpak и добавить тесты native text path, empty PDF fallback contract и graceful error handling. OCR-тесты сделать условными, если Tesseract недоступен в окружении разработки.

Критический файл: `src/src/model/extractor.rs`; также `src/Cargo.toml` и Flatpak-модули.

### 4. Реализовать корректный поиск и worker IPC

- В `model/search.rs` сделать case-insensitive substring matching с Unicode-безопасным сопоставлением: нельзя использовать байтовые offsets от lowercased строки для срезов исходной строки. Формировать до 20 символов контекста до и после совпадения, сохраняя точный matched substring.
- Покрыть тестами пустой запрос, несколько совпадений, совпадение на границе, Unicode и повторяющиеся/перекрывающиеся случаи.
- Добавить worker-протокол: `Scan { root }`, `QueryChanged { query }`, `Open/Shutdown` и события `ScanStarted/Progress/Completed/Error`, `SearchStarted/Progress/Completed/Cancelled/Error`.
- Worker должен жить в выделенном Tokio runtime, отменять предыдущий debounce/search через `CancellationToken`, ждать 300 ms после последнего ввода, проверять отмену между файлами и выполнять blocking extraction в `spawn_blocking` с проверками до/после. Старый поиск не имеет права отправить результаты после нового query.
- Все сообщения worker → UI доставлять через безопасный канал/`ComponentSender`; GTK state менять только в main loop. Пустой запрос должен немедленно очистить результаты и не запускать OCR.

Критические файлы: `src/src/model/actions.rs`, `src/src/model/search.rs`, `src/src/app.rs`; новый модуль `src/src/worker.rs` или `src/src/model/worker.rs`.

### 5. Исторический план UI, portal и persistence

- Реализовать левую панель: абсолютный каталог, количество файлов, относительные пути, кнопку выбора каталога и deterministic activation по `PathBuf`.
- Реализовать правую панель: поле поиска, статус/spinner/success/error, результаты с bold/clickable path и визуальным выделением совпадения, кнопку pagination по 100 элементов.
- Выбор каталога выполнять через `ashpd` FileChooser Portal с корректным `WindowIdentifier`; не хардкодить доступ к host filesystem. Начальный каталог — `/home/documents`.
- Сохранить каталог в GSettings: добавить schema `org.gtk.DocumentSearcher` с ключом path, установить/прочитать его при запуске, обновлять после успешного выбора. Добавить schema compilation/install в `build.rs`/Flatpak sources.
- Открытие файла выполнять через `gio::AppInfo::launch_default_for_uri` с корректным URI-экранированием; не строить URI через `display()` и не использовать текущую рабочую директорию для результата.
- Удалить все no-op forward/update handlers и несоответствующие `AppInput`/`AppMsg`; UI должен показывать ошибки без panic.

Критические файлы: `src/src/app.rs`, `src/src/widgets/sidebar.rs`, `src/src/widgets/search_bar.rs`, `src/src/widgets/results_list.rs`, `src/src/utils.rs`, `src/data/resources/window.blp`; новый `src/data/org.gtk.DocumentSearcher.gschema.xml`.

### 6. Довести Flatpak и документацию до воспроизводимой сборки

- Исправить manifest `src/flatpak/org.gtk.DocumentSearcher.json`: единый app-id, binary name, зависимости Leptonica/Tesseract/tessdata, portal permissions, schema/resource installation и Cargo offline source list.
- Сгенерировать `Cargo.lock` и `flatpak/generated-sources.json` только после стабилизации зависимостей; подключить generated sources в module `document-searcher`. Не оставлять фиктивные SHA/URL или несогласованные application ID.
- Обновить `src/BUILD_INSTRUCTIONS.md` и README-команды под фактическое расположение crate (`src/`) и фактические имена артефактов. В метаданных перечислить поддерживаемые форматы из SRS, а не устаревшие RTF/ODT-only.

Критические файлы: `src/flatpak/org.gtk.DocumentSearcher.json`, `src/BUILD_INSTRUCTIONS.md`, `README.md`, desktop/metainfo resources.

## Проверка

1. `cargo fmt --check` и `cargo clippy --all-targets --all-features -- -D warnings` из каталога `src`.
2. Unit/integration tests для scanner, extractor, search и cancellation; отдельно проверить обработку invalid UTF-8, hidden/symlink/50 MiB, пустого запроса и rapid typing.
3. На Linux/ALT Linux: `cargo build --release`, запуск приложения, выбор каталога через portal, сканирование, поиск, отмена старого запроса, открытие файла и pagination.
4. Проверить persistence после перезапуска и визуальные статусы spinner/success/error.
5. Сгенерировать lock/source manifest и выполнить `flatpak-builder --force-clean --user ...`; затем `flatpak run org.gtk.DocumentSearcher`. Если сборка/Flatpak недоступны на текущей Windows-среде, честно зафиксировать это ограничение и выполнить доступные статические проверки, не выдавая их за end-to-end validation.
6. Перед завершением проверить `git diff` и не затрагивать пользовательские изменения, связанные с переносом спецификаций.
