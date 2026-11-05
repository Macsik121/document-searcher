# Document Searcher — postmortem текущего этапа

Дата отчёта: 2026-11-05

## 1. Цель этапа

Целью этапа было сопоставить текущий репозиторий с документами в `docs/specs/`, старым `document-searcher-implementation-plan.md` и планом разработки, а затем довести проект до проверяемого MVP. В ходе работы выяснилось, что core-часть уже была существенно переработана, а desktop-часть оставалась незавершённым каркасом. Поэтому основное внимание было уделено сохранению работоспособного core и фиксации границ между core, CLI и будущим GTK-приложением.

## 2. Что было обнаружено в начале

- Спецификации были перенесены из `src/specs/` в `docs/specs/`; перенос является пользовательским изменением и не должен откатываться.
- `README.md` уже правильно предупреждал, что core/CLI можно проверять на Windows, а GTK, portal, OCR и Flatpak требуют Linux.
- `src/src/lib.rs`, core-модули и Windows-friendly CLI уже присутствовали.
- Старый план реализации содержал устаревшие решения: `tesseract-rs`, RTF/ODT, неоднозначные GTK/Relm4 API, старые имена application id и недоведённые Flatpak шаги.
- `app.rs` и widgets были не синхронизированы с текущим `AppState` и `AppMsg`: встречались поля и события, которых больше нет, вызовы `tokio::spawn` без гарантированного Tokio runtime, no-op forward handlers и ошибочный debounce-код.

## 3. Что уже реализовано в проекте к концу этапа

### Документация и идентификаторы

- Актуальные спецификации находятся в `docs/specs/`.
- `README.md` описывает Windows core workflow и ограничение по Linux desktop.
- Desktop metadata, GResource и Flatpak manifest частично приведены к application id `org.gtk.DocumentSearcher`, хотя остатки старых имён в путях файлов ещё требуют унификации.

### Core scanner

`src/src/model/scanner.rs` реализует:

- рекурсивное сканирование через `WalkDir`;
- запрет следования symbolic links;
- пропуск скрытых файлов и каталогов на любом уровне;
- список форматов из SRS: TXT, Markdown, Rust, TOML, JSON, XML, HTML, CSS, JavaScript, TypeScript, PDF, DOCX, PNG, JPG/JPEG, TIFF и BMP;
- сравнение расширений без учёта регистра;
- ограничение одного файла 50 MiB;
- продолжение обхода после ошибки отдельного entry;
- детерминированную сортировку относительных путей;
- `ScanReport` со счётчиками oversized и unreadable entries.

Добавлены тесты для расширений, hidden directories, сортировки, лимита размера и symlink policy.

### Extraction

`src/src/model/extractor.rs` реализует composite extraction:

- текстовые форматы читаются как UTF-8;
- invalid UTF-8 возвращается как ошибка отдельного файла;
- DOCX читается из ZIP и `word/document.xml` через XML parser;
- PDF сначала проходит native text-layer extraction через `pdf-extract`;
- OCR не используется для обычных TXT/DOCX/PDF с текстовым слоем;
- изображения требуют feature `ocr` и используют `leptess`;
- scanned PDF использует OCR fallback через `pdftoppm`, если включён `ocr`;
- неподдержанные и повреждённые файлы не должны останавливать общий поиск.

### Search

`src/src/model/search.rs` реализует:

- case-insensitive substring matching;
- Unicode-safe folding через массив folded characters и source spans;
- получение точного фрагмента исходного текста, а не среза lowercased-строки;
- до 20 символов контекста до и после совпадения;
- сохранение `before`, точного `matched`, `after` и полного `snippet`;
- пропуск файлов, для которых extraction завершился ошибкой.

Добавлены тесты для регистра, Unicode offsets, границ контекста, повторяющихся совпадений, пустого запроса и ошибок extraction.

### State и worker

- `AppState` содержит путь каталога, список файлов, query, results, status, error и pagination по 100 результатов.
- В state добавлен `AppSettings` с default path `/home/documents` и счётчик unreadable entries.
- `set_scan_path` сбрасывает старый индекс и результаты.
- `src/src/model/worker.rs` содержит Tokio worker с командами Scan/QueryChanged/Shutdown.
- Для поиска реализованы 300 ms debounce, `CancellationToken`, generation checks против устаревших результатов, `spawn_blocking` для extraction и progress events.
- Пустой query возвращает пустой результат без запуска extraction.
- Добавлены async-тесты scan → search, empty query и superseding query.

### Сборка core

- В `Cargo.toml` desktop/OCR dependencies остаются optional.
- Исправлена ошибка feature-конфигурации Relm4: удалён несуществующий feature `gtk4-0-8`.
- `Cargo.lock` был сгенерирован Cargo при разрешении зависимостей.

## 4. Что проверено и что не удалось проверить

Была выполнена попытка запустить core-тесты на Windows с установленным Rust toolchain.

Обнаружено:

- Cargo и Rust доступны в `C:\Users\максим\.cargo\bin`.
- В текущем окружении `cargo` не был добавлен в PATH автоматически; с временным добавлением PATH команда запускается.
- После исправления Relm4 feature Cargo начал разрешать и скачивать зависимости.
- Сборка остановлена политикой управления приложениями Windows при запуске Cargo build scripts:

```text
Политика управления приложениями заблокировала этот файл. (os error 4551)
```

Ошибка возникла при запуске build scripts `anyhow` и `bzip2-sys`, поэтому результат `cargo test` нельзя считать ни успешным, ни провалом тестов проекта: тестовый бинарник не был собран. Причина относится к политике Windows/AppLocker/антивирусу, а не к assertion в тестах.

Linux/ALT Linux GTK-сборка, OCR и Flatpak в этом окружении не выполнялись.

## 5. Что осталось до рабочего core MVP

1. Проверить `cargo fmt`, `cargo test` и `cargo clippy` в окружении, где Windows policy не блокирует build scripts, либо в ALT Linux VM.
2. Добавить/проверить тесты для exact 50 MiB boundary, вложенных hidden paths, unreadable entries и дополнительных Unicode case-folding случаев.
3. Уточнить contract для empty-PDF fallback и условные OCR-тесты.
4. Проверить, что default OCR language совпадает с реально установленными tessdata. Сейчас extractor по умолчанию ожидает `eng+rus`, а manifest содержит только `eng.traineddata`.

## 6. Что осталось до desktop MVP

Desktop-часть пока нельзя считать готовым приложением. Необходимо:

- переписать/согласовать `src/src/app.rs` с текущим state и worker API;
- запускать GTK и Tokio через единый lifecycle, не вызывая `tokio::spawn` без runtime;
- подключить worker IPC к Relm4 main loop;
- убрать дублирование Blueprint layout и inline Relm4 `view!`;
- исправить `search_bar.rs`: текущий debounce использует некорректный async/sync код и неверную задержку;
- заменить индексную активацию строк стабильным `PathBuf`/модельным идентификатором;
- реализовать portal chooser с корректным `WindowIdentifier` текущего окна;
- добавить GSettings schema `org.gtk.DocumentSearcher`, чтение пути при старте и сохранение после успешного выбора;
- реализовать статус scanning/searching/success/error, подсветку exact match и pagination в реальном UI;
- открывать файлы через корректный GIO/portal URI API с escaping, а не через `file://` + `display()`;
- исправить и проверить resource loading: относительный путь к `.gresource` не подходит как универсальный runtime-механизм для установленного Flatpak.

## 7. Что осталось до Flatpak-релиза

- унифицировать физические имена desktop/metainfo/manifest файлов с app id `org.gtk.DocumentSearcher` либо последовательно оставить имена файлов `org.example...` только как внутренние пути без расхождений в содержимом;
- установить GSettings schema в manifest;
- определить корректный способ сборки и установки Blueprint/GResource;
- добавить/сгенерировать `flatpak/generated-sources.json` после фиксации зависимостей;
- выполнить offline Flatpak build без фиктивных SHA/URL;
- добавить русскую tessdata либо сменить default OCR language на реально поставляемый `eng`;
- проверить permissions, portal access и запуск через `flatpak run org.gtk.DocumentSearcher`;
- проверить desktop entry, AppStream metadata и installed resources на ALT Linux.

## 8. Итог

На текущем этапе проект имеет полезное и достаточно хорошо тестируемое ядро поиска, scanner, native extraction, worker prototype и CLI. Это уже не пустой каркас. Однако GTK-приложение ещё не является интегрированным runnable MVP: между текущим core/worker и старым Relm4 UI отсутствует безопасный bridge, persistence и законченный Flatpak pipeline.

На Windows разумно продолжать разработку и тестирование core/CLI. Финальную desktop/OCR/Flatpak интеграцию следует проверять в ALT Linux VM после установки GTK4, Libadwaita, Blueprint Compiler, GLib development tools, portal, Tesseract/Leptonica и Flatpak Builder.



Работа завершена. Финальная сводка.

Что сделано в этом этапе

Core (Windows):
- model/state.rs — добавлен AppSettings с default /home/documents, счётчик unreadable_entries, метод set_scan_path, тест на сброс при смене каталога.
- model/actions.rs — WorkerEvent/AppMsg расширены: ScanProgress, unreadable_entries в ScanCompleted, query в SearchCancelled.
- model/worker.rs — ScanCompleted передаёт unreadable_entries.
- Cargo.toml — исправлена ошибка сборки: убран несуществующий feature gtk4-0-8 у Relm4.
- Cargo.lock — сгенерирован Cargo.

Документация:
- docs/postmortem.md — полный разбор: что было, что реализовано, что не проверено, что осталось до core/desktop/Flatpak MVP.
- docs/ALT-LINUX-RUN.md — инструкции по запуску core/CLI, OCR и будущей desktop/Flatpak сборки на ALT Linux, плюс диагностика ошибки Windows policy 4551.
- README.md — добавлены точные команды с --manifest-path и временным PATH для Cargo.

Что блокировало проверку

cargo test на Windows не завершился: политика управления приложениями блокирует запуск Cargo build scripts (os error 4551 для anyhow и bzip2-sys). Это не падение тестов проекта — бинарник не был собран. Проверить нужно в ALT Linux VM или разрешить build scripts в Windows policy.

Команды для самостоятельного запуска

На Windows (после установки Rust, новый терминал):
$env:Path = "$env:USERPROFILE\.cargo\bin;" + $env:Path
cargo test --manifest-path .\src\Cargo.toml --lib --no-default-features
cargo run --manifest-path .\src\Cargo.toml --no-default-features -- scan <directory>
cargo run --manifest-path .\src\Cargo.toml --no-default-features -- search <directory> <query>

На ALT Linux — см. docs/ALT-LINUX-RUN.md (core/CLI готов; desktop/Flatpak требует интеграции, описанной в docs/postmortem.md).

Что осталось (не этот этап)

Desktop MVP: bridge worker↔Relm4, GSettings schema, portal chooser, исправление search_bar.rs debounce, открытие файлов через GIO URI, единый Blueprint layout. Flatpak: унификация app-id, generated-sources, schema/resource install, tessdata eng+rus. Всё это зафиксировано в postmortem.

Перенос src/specs → docs/specs сохранён, пользовательские документы не тронуты.