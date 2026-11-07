# Document Searcher — Postmortem сессии 2026-11-07

## 1. Контекст и цели

Продолжение разработки проекта Document Searcher. Цель сессии — выполнить подготовительные изменения по плану `~/.claude/plans/whimsical-leaping-lemon.md`: унифицировать metadata, исправить RuntimeBridge shutdown, убрать неиспользуемый код, подготовить репозиторий к переносу в ALT Linux VM.

## 2. Выполненные изменения

### 2.1. Metadata унификация

**Удалены:**
- `src/data/org.example.DocumentSearcher.desktop`
- `src/data/org.example.DocumentSearcher.metainfo.xml`
- `src/flatpak/org.example.DocumentSearcher.json`

**Переименованы/созданы:**
- `src/data/org.gtk.DocumentSearcher.desktop`
- `src/data/org.gtk.DocumentSearcher.metainfo.xml`
- `src/flatpak/org.gtk.DocumentSearcher.json` — единственный актуальный manifest

Все ссылки теперь используют `org.gtk.DocumentSearcher` как application ID.

### 2.2. RuntimeBridge shutdown lifecycle

**Файл:** `src/src/app.rs`

- Удалён неиспользуемый метод `shutdown()`, который создавал новый канал без receiver
- Оставлен единственный путь завершения: `Drop` отправляет `WorkerCommand::Shutdown` через `blocking_send` в отдельном OS-потоке, затем вызывает `join()` на runtime thread
- Это предотвращает блокировку GTK thread при полной очереди команд и даёт worker'у возможность graceful exit

### 2.3. Build.rs Blueprint cleanup

**Файл:** `src/build.rs`

- Удалён `cargo:rerun-if-changed=data/resources/window.blp`
- Удалён вызов `blueprint-compiler` для `window.blp → window.ui`
- Оставлена только компиляция `gresource.xml` (содержит только `styles.css`) и GSettings schema

### 2.4. GResource текущее состояние

- GResource компилируется в `$OUT_DIR/org.gtk.DocumentSearcher.gresource`
- Runtime загружает ресурс по пути из `env!("DOCUMENT_SEARCHER_GRESOURCE")`
- **Проблема:** путь `OUT_DIR` недоступен после упаковки Flatpak
- **Решение в плане:** встроить ресурс в бинарник через compile-time bytes или установить в `/app/share/...`

## 3. Текущее состояние git

```
M docs/specs/requirements-spec.md      — пользовательские изменения, сохранить
M src/Cargo.toml
M src/build.rs
M src/data/resources/gresource.xml
M src/src/app.rs
M src/src/main.rs
M src/src/widgets/mod.rs
D src/data/org.example.DocumentSearcher.desktop
D src/data/org.example.DocumentSearcher.metainfo.xml
D src/flatpak/org.example.DocumentSearcher.json
?? docs/postmortem-session-2026-11-07.md
?? src/data/org.gtk.DocumentSearcher.desktop
?? src/data/org.gtk.DocumentSearcher.gschema.xml
?? src/data/org.gtk.DocumentSearcher.metainfo.xml
?? src/flatpak/org.gtk.DocumentSearcher.json
```

## 4. Оставшиеся задачи по плану

### 4.1. Требуют Linux (ALT Linux VM)

- [ ] Проверить `cargo build --features desktop`
- [ ] GUI smoke tests: запуск, folder chooser, GSettings persistence, scan, search, pagination
- [ ] Сгенерировать `flatpak/generated-sources.json`
- [ ] Flatpak build и `flatpak run org.gtk.DocumentSearcher`
- [ ] Проверить что schema находится через `gio::Settings` в Flatpak
- [ ] Проверить portal permissions

### 4.2. Можно сделать на Windows

- [ ] Удалить неиспользуемый `src/src/utils.rs` (не подключён в `lib.rs`, содержит stale portal helpers)
- [ ] Удалить `src/data/resources/window.blp` (не используется, UI процедурный)
- [ ] Реализовать install-safe GResource: embedded bytes или `/app/share` путь
- [ ] Обновить `README.md` и `src/BUILD_INSTRUCTIONS.md`
- [ ] Создать `docs/architecture/` с Mermaid-диаграммами
- [ ] Выполнить `cargo fmt --check`, core tests, clippy

## 5. Известные проблемы

1. **GResource install-time risk:** Runtime загружает ресурс из `OUT_DIR`, который не существует в установленном Flatpak. Нужно встроить ресурс или установить в стабильный путь.

2. **GSettings dev fallback:** `load_settings()` использует только default schema source. В native dev-запуске schema может не найтись без явного указания `GSETTINGS_SCHEMA_DIR`. Flatpak manifest устанавливает schema корректно.

3. **OCR language mismatch:** Extractor default `eng+rus`, Flatpak поставляет только `eng`. Manifest задаёт `DOCUMENT_SEARCHER_OCR_LANG=eng`.

## 6. Архитектура (для справки)

- **Entry points:** CLI (no feature) / GTK desktop (`desktop` feature)
- **UI:** Процедурный GTK4/Libadwaita, не Relm4
- **Worker:** Отдельный Tokio runtime thread, сообщение через `mpsc` channels
- **Contracts:** `WorkerCommand`/`WorkerEvent` в `model/actions.rs`
- **State:** `AppState` в `model/state.rs`, мутации только в GLib main loop

## 7. Следующий шаг

Продолжить по плану с этапа 3 (install-safe GResource), затем архитектурная документация и Windows-проверки. Перенос в ALT Linux VM — после коммита подготовленных изменений.

---

**План:** `~/.claude/plans/whimsical-leaping-lemon.md`
