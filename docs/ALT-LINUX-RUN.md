# Запуск Document Searcher на ALT Linux

Этот документ описывает запуск текущей версии проекта. На текущем этапе надёжно запускается core/CLI. GTK/Libadwaita-приложение и Flatpak пока требуют отдельного этапа интеграции, перечисленного в [`postmortem.md`](postmortem.md).

## 1. Подготовить исходники

Передайте репозиторий в ALT Linux VM любым удобным способом. Перейдите в crate:

```sh
cd document-searcher/src
```

Проверьте версии:

```sh
rustc --version
cargo --version
```

Репозиторий содержит `rust-toolchain.toml` и просит Rust 1.78.0. Если в VM установлен только системный `rustc/cargo`, можно временно использовать его для core, но при расхождении версий сначала установите rustup toolchain 1.78.0.

## 2. Проверить core без GTK/OCR

```sh
cd document-searcher/src
cargo fmt --check
cargo test --lib --no-default-features
cargo clippy --lib --tests --no-default-features -- -D warnings
```

Если Cargo ещё не скачал зависимости, обычная команда `cargo test` потребует доступ к crates.io. Для offline-сборки Flatpak сначала должен быть создан полный `flatpak/generated-sources.json`.

## 3. Запустить CLI scan

```sh
cargo run --no-default-features -- scan /home/documents
```

Команда выводит абсолютный каталог, количество найденных файлов, счётчики пропущенных oversized/unreadable entries и отсортированные относительные пути.

## 4. Запустить CLI search

```sh
cargo run --no-default-features -- search /home/documents поисковый-запрос
```

Для запроса с пробелами используйте кавычки:

```sh
cargo run --no-default-features -- search /home/documents "служебная записка"
```

Проверить справку:

```sh
cargo run --no-default-features -- --help
```

## 5. Сборка исполняемого файла

```sh
cargo build --release --no-default-features
./target/release/document-searcher --help
./target/release/document-searcher scan /home/documents
./target/release/document-searcher search /home/documents "текст"
```

Исполняемый файл находится в `src/target/release/document-searcher`.

## 6. Проверка OCR

OCR не входит в default build. Для него нужны системные библиотеки Tesseract/Leptonica, `pdftoppm` и языковые данные. Минимально проверьте наличие инструментов:

```sh
tesseract --version
pdftoppm -v
pkg-config --modversion tesseract lept
```

Для текущего extractor по умолчанию используется язык `eng+rus`. Поэтому должны быть доступны обе модели, либо перед запуском задайте язык, для которого установлена tessdata:

```sh
export DOCUMENT_SEARCHER_OCR_LANG=eng
cargo test --features ocr --lib
cargo run --features ocr -- search /path/to/documents "query"
```

Если Tesseract и Leptonica установлены в нестандартном месте, дополнительно настройте `PKG_CONFIG_PATH`, `TESSDATA_PREFIX` и переменные сборки вашей системы.

## 7. GTK desktop build — текущий статус

Текущий репозиторий содержит GTK/Relm4/Libadwaita каркас, но его нельзя запускать как готовое desktop-приложение: `app.rs`, widgets, worker IPC, Blueprint/GResource, portal и GSettings пока не сведены в единый runnable pipeline. Не выдавайте успешную core CLI-сборку за успешную GUI-сборку.

После завершения desktop-интеграции ожидаемые команды будут такими:

```sh
cargo check --features desktop
cargo build --release --features desktop
cargo clippy --all-targets --all-features -- -D warnings
```

Перед этим должны быть установлены GTK4, Libadwaita, Blueprint Compiler, GLib development tools, ashpd/portal runtime, pkg-config и системные native dependencies.

## 8. Flatpak — что нужно после интеграции GUI

В каталоге `src/flatpak/` находится черновой manifest. Перед сборкой необходимо:

1. исправить и проверить единый app-id `org.gtk.DocumentSearcher`;
2. добавить установку GSettings schema и ресурсов;
3. согласовать resource loading с установленными Flatpak paths;
4. выбрать и поставить `eng`/`rus` tessdata в соответствии с `DOCUMENT_SEARCHER_OCR_LANG`;
5. создать настоящий `flatpak/generated-sources.json` без placeholder SHA;
6. выполнить сборку из Linux/ALT Linux.

После этого ожидаемая команда:

```sh
cd document-searcher/src
flatpak-builder --force-clean --user --install-deps-from=flathub \
  flatpak/build-dir flatpak/org.gtk.DocumentSearcher.json
flatpak run org.gtk.DocumentSearcher
```

Сейчас имя manifest-файла в репозитории ещё требует унификации с app-id; перед использованием проверьте фактический путь командой `ls flatpak`.

## 9. Диагностика Windows-проверки

Если на Windows Cargo сообщает:

```text
Политика управления приложениями заблокировала этот файл. (os error 4551)
```

это означает, что Windows policy/AppLocker/антивирус заблокировал запуск Cargo build script. Это не результат падения теста проекта. Запустите те же команды в ALT Linux VM или настройте разрешение для build scripts в Windows согласно политике вашей системы.
