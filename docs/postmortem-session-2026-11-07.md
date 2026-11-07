# Document Searcher — postmortem сессии 2026-11-07

## Продолжение работ 2026-11-08

### Что сделано в предыдущей сессии

- Application ID приведён к `org.gtk.DocumentSearcher`; старые desktop/metainfo/Flatpak имена удалены, создан единственный актуальный manifest `src/flatpak/org.gtk.DocumentSearcher.json`.
- `RuntimeBridge` завершает worker через `Drop`: отправка `Shutdown` и join runtime thread выполняются в отдельном потоке.
- Blueprint-компиляция удалена из `build.rs`; ресурсный XML содержит только `styles.css`, а schema собирается GLib tooling на Linux.
- Пользовательский diff `docs/specs/requirements-spec.md` необходимо сохранять без изменений.

### Windows core/CLI validation

Все команды выполнены на Windows только с `--no-default-features`; desktop/OCR features не запускались:

- `cargo test --manifest-path .\\src\\Cargo.toml --lib --no-default-features` — 18 passed, 0 failed. Включая пять регрессионных search-тестов: case-insensitive, empty query, 20-character context, Unicode offsets и non-overlapping matches.
- `cargo fmt --check --manifest-path .\\src\\Cargo.toml` — успешно, без вывода.
- `cargo clippy --manifest-path .\\src\\Cargo.toml --lib --tests --no-default-features -- -D warnings` — успешно, предупреждений нет.
- CLI smoke `scan` на временном каталоге — найден один `sample.txt`.
- CLI smoke `search ... needle` на том же каталоге — найден один файл; контекст: `A searchable needle in UTF-8 text.`.

Результаты подтверждают core/CLI на Windows. Они не подтверждают native GUI, portal, GSettings runtime, OCR или Flatpak.

### Изменения реализации и документации 2026-11-08

- Runtime GResource теперь читается через `include_bytes!(env!("DOCUMENT_SEARCHER_GRESOURCE"))`, преобразуется через `glib::Bytes::from_static` и `gio::Resource::from_data`, затем регистрируется до запуска UI. Runtime не загружает файл из `OUT_DIR`; отдельный `.gresource` в Flatpak не устанавливается. CSS path `/org/gtk/DocumentSearcher/styles.css` сохранён.
- Удалены stale `src/src/utils.rs` и `src/data/resources/window.blp`; комментарий в `app.rs` описывает procedural UI.
- Синхронизированы `README.md`, `src/BUILD_INSTRUCTIONS.md`, `docs/ALT-LINUX-RUN.md` и актуальные deployment notes.
- Созданы `docs/architecture/README.md`, `component-architecture.md`, `scan-search-sequence.md`, `user-flow.md`, `deployment.md`, `state-and-data-model.md`, `assumptions-gaps.md`.

### Не проверено: требуется ALT Linux VM

- `cargo build --release --features desktop`, запуск GUI и CSS lookup из embedded resource.
- Portal folder chooser, доступ к выбранной директории, открытие файлов через GIO/portal.
- GSettings schema discovery и сохранение/восстановление `scan-path`.
- Image/scanned-PDF OCR и соответствие manifest language `eng` против extractor default `eng+rus`.
- Генерация `flatpak/generated-sources.json`, `flatpak-builder`, permissions и `flatpak run org.gtk.DocumentSearcher`.

## Ограничения и следующий этап

На Windows разрешены только fmt, core tests, Clippy и CLI smoke с `--no-default-features`. Все desktop, portal, GSettings runtime, OCR и Flatpak результаты необходимо получить отдельно в ALT Linux VM. Коммит не создавался.

**Планы:** `~/.claude/plans/document-searcher-piped-cat.md` и
`~/.claude/plans/document-searcher-sharded-otter.md`.
