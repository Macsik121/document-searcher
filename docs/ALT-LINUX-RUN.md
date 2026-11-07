# Запуск Document Searcher на ALT Linux

Это руководство разделяет проверку core/CLI и desktop/Flatpak. Windows core
checks не проверяют GTK, portal, GSettings, OCR или Flatpak. Эти функции
проверяются только в ALT Linux VM.

## 1. Подготовить исходники

Передайте репозиторий в ALT Linux VM и перейдите в crate:

```sh
cd document-searcher/src
rustc --version
cargo --version
```

Репозиторий содержит `rust-toolchain.toml`. Если системный Rust отличается от
закреплённой версии, используйте подходящий toolchain из rustup.

## 2. Проверить core и CLI

```sh
cargo fmt --check
cargo test --lib --no-default-features
cargo clippy --lib --tests --no-default-features -- -D warnings
cargo run --no-default-features -- scan /home/documents
cargo run --no-default-features -- search /home/documents "поисковый запрос"
```

CLI выводит число найденных файлов, счётчики пропусков и относительные пути.
Пустой набор core checks не подтверждает корректность desktop integration.

## 3. Native desktop build и smoke checks

Для desktop сборки нужны GTK4, Libadwaita, GLib development tools
(`glib-compile-resources`, `glib-compile-schemas`), Rust/native dependencies,
Tesseract/Leptonica, `pdftoppm` и portal runtime. Установка Blueprint Compiler
не нужна: UI строится процедурно. `build.rs` автоматически генерирует
GResource и schema при Linux desktop build; CSS GResource встраивается в
binary, runtime не загружает его из `OUT_DIR`.

```sh
cargo build --release --features desktop
cargo run --release --features desktop
```

Проверить отдельно:

- запуск окна и загрузку CSS из embedded GResource;
- начальный каталог `/home/documents`;
- folder chooser select/cancel и доступ к выбранному каталогу;
- сохранение и восстановление GSettings `scan-path`;
- scan/search, отмену предыдущего запроса, результаты и pagination;
- открытие файла через GIO default application/portal;
- OCR изображений и PDF при наличии tessdata.

Все эти результаты фиксировать отдельно от Windows core validation.

## 4. Проверка OCR

Extractor по умолчанию использует `eng+rus`, если не задана
`DOCUMENT_SEARCHER_OCR_LANG`. Текущий Flatpak manifest задаёт `eng` и
поставляет только English tessdata. Это известное расхождение: не менять язык
или состав tessdata до проверки конкретных моделей в ALT Linux VM.

Для диагностики в VM:

```sh
tesseract --version
pdftoppm -v
pkg-config --modversion tesseract lept
```

## 5. Flatpak build/install/run

Единственный актуальный manifest:
`flatpak/org.gtk.DocumentSearcher.json`. После стабилизации lockfile
сгенерируйте Cargo source manifest в Linux VM и затем соберите пакет:

```sh
cargo flatpak-generator --output flatpak/generated-sources.json
flatpak-builder --force-clean --install-deps-from=flathub \
  flatpak/build-dir flatpak/org.gtk.DocumentSearcher.json
flatpak-builder --force-clean --user --install \
  flatpak/build-dir flatpak/org.gtk.DocumentSearcher.json
flatpak run org.gtk.DocumentSearcher
```

Не создавайте `generated-sources.json` вручную на Windows и не считайте
Flatpak проверенным до завершения `flatpak-builder` и `flatpak run` в VM.
Manifest устанавливает desktop entry, AppStream metadata и GSettings schema;
отдельная установка `.gresource` не нужна, поскольку CSS embedded в binary.
Проверьте portal permissions, schema discovery, ресурс CSS, открытие файлов,
OCR runtime и доступ к пользовательскому каталогу внутри sandbox.

## 6. Диагностика Windows-проверки

Если Windows сообщает, что политика управления приложениями заблокировала
Cargo build script (например, `os error 4551`), это означает, что команда не
была проверена на этой машине. Зафиксируйте точный текст ошибки и выполните
разрешённые core checks в ALT Linux VM. Не запускайте desktop feature на
Windows.
