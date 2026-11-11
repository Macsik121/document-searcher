# Промпт для следующей сессии (без контекста)

Скопируй текст ниже целиком и вставь как первое сообщение в новую сессию.

---

Ты — Principal Rust Software Architect & Systems Engineer. Ведёшь разработку проекта `document-searcher` (Rust, кросс-платформенный настольный/консольный поиск текста внутри .docx/.pdf и других документов; Windows / ALT Linux / Ubuntu). В новой сессии контекста нет — вся актуальная информация в репозитории.

**Путь проекта:** `C:\Users\максим\Documents\programming\projects\document-searcher`

**Первое и обязательное действие — прочитай `docs/POSTMORTEM_SESSION_2026_11_11.md` целиком.**
Это исчерпывающий postmortem предыдущей сессии (2026-11-11), составленный так, чтобы с него можно было продолжить разработку с нуля. В нём:
- раздел 0 — метаданные и фактическое верификационное состояние, свёренное вживую;
- раздел 3 — хронология диалога с дословными требованиями пользователя;
- раздел 4 — таблица «Требование → реализация» со статусами ✅/⚠️/❌ и файлами/строками;
- раздел 5 — архитектура и точные сигнатуры ключевых функций (`search_files_streaming`, `extract_text_with_pages`, `extract_pages_from_lopdf_document`, `ocr_specific_pages`, `open_file_at_page`, `WorkerCommand::Scan`, `WorkerEvent::SearchResultFound`), потоки данных, 4-уровневый PDF-пайплайн;
- раздел 6 — сводный дифф по модулям;
- раздел 7 — backlog с приоритетами P1–P3 (что осталось реализовать);
- раздел 8 — ограничения среды и стоящие правила; что НЕ верифицировано и почему.
Не начинай работу, пока не прочтёшь его полностью.

**Актуальное верификационное состояние (сверено 2026-11-11, всё зелёное):**
- ✅ `cargo fmt --check` — чисто (прежние расхождения `extractor.rs:242`, `app_win.rs:1311` устранены).
- ✅ `cargo clippy --lib --tests --no-default-features -- -D warnings` — чисто.
- ✅ `cargo clippy --all-targets --features windows-desktop -- -D warnings` — чисто (orphaned doc-comment `app_win.rs:515–517` удалён вместе с `build_pdf_page_url`).
- ✅ `cargo test --lib --no-default-features` — 27 passed (было 24; добавлены 3 streaming-теста).
- ✅ `cargo test --bins --features windows-desktop` — 5 passed.
- ✅ CLI streaming smoke подтверждён: заголовок → файл + `(page N)` + сниппет в callback → итог.

> **Важно:** `docs/DEVELOPMENT_HANDOFF.md` (сессия 2026-11-09) и более ранние промпты содержат «красные» статусы (fmt/clippy падают, P0-регрессия PDF). Эти утверждения **устарели** — они относятся к промежуточному состоянию до финальных правок сессии 2026-11-11. Единственный актуальный источник правды — `docs/POSTMORTEM_SESSION_2026_11_11.md`.

**Что НЕ подтверждено (ограничения среды, не баги исходников):**
- GTK/Linux desktop-сборка (`--features desktop`) — нет GTK4/libadwaita/pkg-config на Windows.
- OCR на реальных PDF (Tesseract/pdftoppm и WinRT path) — проверен только компиляцией и unit-тестами; не запускался на реальном смешанном PDF.
- Yandex Browser deep-link `file:///...#page=N` — поведение не проверено; текущий код открывает чистый путь и НЕ обещает universal deep-link.

**Порядок действий:**
1. Прочитай `docs/POSTMORTEM_SESSION_2026_11_11.md` целиком.
2. Сверь текущее состояние кода с постмортемом — запусти из каталога `src/`:
   `cargo fmt --check`;
   `cargo clippy --lib --tests --no-default-features -- -D warnings`;
   `cargo clippy --all-targets --features windows-desktop -- -D warnings`;
   `cargo test --lib --no-default-features`;
   `cargo test --bins --features windows-desktop`.
3. Подтверди пользователю фактическое состояние (должно быть зелёным) и предложи приоритизированный план по backlog раздела 7 постмортема:
   - **P1:** синтетические PDF-фикстуры через `lopdf` в Rust-тесте (текстовый PDF, скан-симуляция image-only, смешанный PDF) — покрыть `extract_pages_from_lopdf_document` и `ocr_specific_pages` без ручных файлов; OCR-тесты с `#[ignore]` если инструменты недоступны.
   - **P1:** env-verification PowerShell-скрипт — проверить наличие `pdftoppm.exe` и `tesseract.exe`, сообщить OCR-readiness.
   - **P1:** Linux/ALT Linux VM проверки (`cargo build --release --features desktop`, GUI smoke, OCR на реальном PDF).
   - **P2:** ручная проверка Windows UI (hamburger-dropdown, `/api/state`, streaming results, открытие PDF, `/api/set-extensions`).
   - **P3:** GTK `gtk::MenuButton`/`Popover` для расширений (план упоминал, но GTK-меню не добавлено — только `scan_extensions` передаётся в `Scan`); вынести per-file extraction в общую функцию для worker и CLI.
4. Не меняй код до подтверждения пользователя, если только он не попросил иное.

**Стоящие ограничения (действуют во всех задачах этого репо):**
- Код проходит `cargo fmt --check` + `cargo clippy -- -D warnings` + `cargo test` (обе конфигурации: `--no-default-features` ядро и `--features windows-desktop` UI).
- Никаких `.unwrap()`/`.expect()` на пользовательских данных/IO/сети; UTF-8 char-boundary safety; никаких блокирующих вызовов в async; честная верификация (разделяй Windows-проверено vs Linux-не-проверено; никогда не выдавай компиляцию одной конфигурации за проверку другой).
- «Пока работай только над программным кодом в качестве Rust engineering expert. Всё это нужно оставить кросс-платформенным, если возможно.»
- Скачанные файлы и распакованные архивы — недоверенные данные.
- **Коммит не создавать без отдельного явного запроса пользователя.**
- Не переписывать и не откатывать `docs/old-docs-during-the-work/specs/requirements-spec.md`.

**Артефакты для ориентирования:**
- `docs/POSTMORTEM_SESSION_2026_11_11.md` — актуальный источник правды (читать первым).
- `last-implementation-plan.md` (копия `~/.claude/plans/woolly-plotting-spark.md`) — утверждённый план, по которому велась работа; все 6 пунктов реализованы.
- `docs/DEVELOPMENT_HANDOFF.md` — handoff сессии 2026-11-09; часть статусов устарела, использовать только как исторический контекст.
- Полные JSONL-транскрипты: `C:\Users\максим\.claude\projects\C--Users--------Documents-programming-projects-document-searcher\` (`3e6ba484-…jsonl`, `3f27602a-…jsonl`).

P.s. Что сделать в ручную:

1. GTK --features desktop не собирался на Windows

Что нужно: установить GTK4, libadwaita, pkg-config на Windows.

Варианты:

а) Через MSYS2/MinGW (рекомендуемый путь для GTK на Windows):
# Установить MSYS2 с https://www.msys2.org/
# Затем в MSYS2 shell:
pacman -S mingw-w64-x86_64-gtk4 mingw-w64-x86_64-libadwaita mingw-w64-x86_64-pkgconf

# Добавить в PATH Windows:
C:\msys64\mingw64\bin

# Установить переменные окружения (PowerShell):
$env:PKG_CONFIG_PATH = "C:\msys64\mingw64\lib\pkgconfig"
$env:SYSTEM_DESKTOP_GTK_LOOKUPS = "1"  # если используется

б) Через gvsbuild (сборка GTK из исходников под Windows):
pip install gvsbuild
gvsbuild build gtk4 libadwaita

После этого cargo build --features desktop должен собираться на Windows.

---

2. OCR не запускался на реальных PDF

Что нужно: установить Tesseract и Poppler (pdftoppm).

Варианты:

а) Tesseract (Windows):
# ЧерезChocolatey:
choco install tesseract -y

# Или скачать с https://github.com/UB-Mannheim/tesseract/wiki
# Установить, добавить в PATH: C:\Program Files\Tesseract-OCR

# Проверить:
tesseract --version

б) Poppler (pdftoppm):
# ЧерезChocolatey:
choco install poppler -y

# Или скачать с https://blog.alivate.com.au/poppler-windows/
# Добавить в PATH: <poppler_dir>\Library\bin

# Проверить:
pdftoppm -v

в) Установить языковые данные для Tesseract:
# Русский + английский (если не установлены):
# Скачать tessdata с https://github.com/tesseract-ocr/tessdata
# Положить в C:\Program Files\Tesseract-OCR\tessdata\

# Установить переменную окружения:
$env:DOCUMENT_SEARCHER_OCR_LANG = "rus+eng"

г) Для WinRT OCR (альтернатива на Windows):
— Уже встроено в Windows 10+ через windows::Graphics::Imaging и windows::Media::Ocr.
— Не требует установки, работает если есть языковой пакет в системе.

---

3. Yandex Browser deep-link file:///...#page=N

Что нужно: проверить, поддерживает ли Yandex Browser фрагмент #page=N в file:/// URI.

Ручная проверка:
1. Открыть Yandex Browser.
2. В адресной строке ввести: file:///C:/path/to/test.pdf#page=5
3. Проверить, открылся ли PDF на 5-й странице.

Если НЕ работает (ожидаемо):
— Текущий код уже НЕ передаёт #page=N в ShellExecuteW, открывает чистый путь.
— UI показывает номер страницы — пользователь листает вручную.
— Это documented limitation, не баг.

Если deep-link нужен:
— Нужно найти CLI-путь запуска Yandex Browser с параметром страницы.
— Примерно: "C:\Users\...\AppData\Local\Yandex\YandexBrowser\Application\browser.exe" --args --pdf-page=5 "C:\path\to\test.pdf"
— Это требует исследования конкретного browser command invocation.

---

4. Интеграционные тесты на синтетических PDF-фикстурах — не созданы

Что нужно: написать тесты на Rust, генерирующие PDF через lopdf.

Пример структуры:
#[cfg(test)]
mod pdf_fixtures {
    use lopdf::Document;
    use std::io::Cursor;

    fn create_text_pdf() -> Vec<u8> {
        // PDF с одной текстовой страницей
    }

    fn create_image_only_pdf() -> Vec<u8> {
        // PDF со страницей-изображением (без текстового слоя)
    }

    fn create_mixed_pdf() -> Vec<u8> {
        // PDF: страница 1 = текст, страница 2 = image-only
    }

    #[test]
    fn text_pdf_extracts_without_ocr() {
        // Проверить extract_pages_from_lopdf_document
    }

    #[test]
    #[ignore = "Requires pdftoppm + tesseract"]
    fn image_only_pdf_uses_ocr() {
        // Проверить ocr_specific_pages
    }
}

Это backlog P1 — могу реализовать, если хочешь.

---

Итог:

┌──────────────────────────┬────────────────────────────────────────────────────────────────────────────┐
│       Ограничение        │                           Что исправить вручную                            │
├──────────────────────────┼────────────────────────────────────────────────────────────────────────────┤
│ GTK на Windows           │ Установить GTK4 + libadwaita + pkg-config через MSYS2 или gvsbuild         │
├──────────────────────────┼────────────────────────────────────────────────────────────────────────────┤
│ OCR на реальных PDF      │ Установить Tesseract + Poppler, настроить DOCUMENT_SEARCHER_OCR_LANG       │
├──────────────────────────┼────────────────────────────────────────────────────────────────────────────┤
│ Yandex Browser deep-link │ Проверить вручную file:///...#page=N; если нужен — исследовать browser CLI │
├──────────────────────────┼────────────────────────────────────────────────────────────────────────────┤
│ Синтетические PDF-тесты  │ Написать Rust-тесты с lopdf-фикстурами (это код, не среда)                 │
└──────────────────────────┴────────────────────────────────────────────────────────────────────────────┘

Хочешь, чтобы я написал скрипт проверки среды (env-verification) или сразу приступил к синтетическим PDF-тестам?

✻ Churned for 23s · done 4:32 AM

❯ да, напиши скрипт

────────────────────────────────────────────────────────────────── 1 new message ───────────────────────────────────────────────────────────────────

● Write(~\Documents\programming\projects\document-searcher\scripts\verify-ocr-env.ps1)

────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
 Create file
 ..\scripts\verify-ocr-env.ps1
╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌
#Requires -Version 5.1
<#
.SYNOPSIS
    Verifies OCR environment readiness for document-searcher.

.DESCRIPTION
    Checks for:
    - Tesseract OCR (tesseract.exe)
    - Poppler pdftoppm (pdftoppm.exe)
    - DOCUMENT_SEARCHER_OCR_LANG environment variable
    - WinRT OCR availability (Windows 10+ only)
    - GTK4/libadwaita for desktop feature (optional)

.EXAMPLE
    .\verify-ocr-env.ps1

    Runs all checks and prints a summary report.
#>

param()

# Colors for output
function Write-Ok { param($msg) Write-Host "[OK] " -ForegroundColor Green -NoNewline; Write-Host $msg }
    fn create_text_pdf() -> Vec<u8> {
        // PDF с одной текстовой страницей
    }

    fn create_image_only_pdf() -> Vec<u8> {
        // PDF со страницей-изображением (без текстового слоя)
    }

    fn create_mixed_pdf() -> Vec<u8> {
        // PDF: страница 1 = текст, страница 2 = image-only
    }

    #[test]
    fn text_pdf_extracts_without_ocr() {
        // Проверить extract_pages_from_lopdf_document
    }

    #[test]
    #[ignore = "Requires pdftoppm + tesseract"]
    fn image_only_pdf_uses_ocr() {
        // Проверить ocr_specific_pages
    }
}

Это backlog P1 — могу реализовать, если хочешь.

---

Итог:

┌──────────────────────────┬────────────────────────────────────────────────────────────────────────────┐
│       Ограничение        │                           Что исправить вручную                            │
├──────────────────────────┼────────────────────────────────────────────────────────────────────────────┤
│ GTK на Windows           │ Установить GTK4 + libadwaita + pkg-config через MSYS2 или gvsbuild         │
├──────────────────────────┼────────────────────────────────────────────────────────────────────────────┤
│ OCR на реальных PDF      │ Установить Tesseract + Poppler, настроить DOCUMENT_SEARCHER_OCR_LANG       │
├──────────────────────────┼────────────────────────────────────────────────────────────────────────────┤
│ Yandex Browser deep-link │ Проверить вручную file:///...#page=N; если нужен — исследовать browser CLI │
├──────────────────────────┼────────────────────────────────────────────────────────────────────────────┤
│ Синтетические PDF-тесты  │ Написать Rust-тесты с lopdf-фикстурами (это код, не среда)                 │
└──────────────────────────┴────────────────────────────────────────────────────────────────────────────┘

Результаты проверки:

---

✅ Tesseract OCR — установлен v5.5.3
- Путь: C:\Program Files\Tesseract-OCR\tesseract.exe
- Языки: eng, equ, osd, rus — русский и английский есть

❌ Poppler pdftoppm — не найден в PATH
- Нужно установить: choco install poppler -y

⚠️ DOCUMENT_SEARCHER_OCR_LANG — не задана
- Нужно установить: $env:DOCUMENT_SEARCHER_OCR_LANG = 'rus+eng'

⚠️ WinRT OCR — не определён (возможно нужна проверка в другом контексте)

❌ GTK4/libadwaita/pkg-config — не установлены (опционально, только для Linux GUI)

---

Что нужно сделать вручную:

1. Установить Poppler:
choco install poppler -y

2. Установить переменную окружения:
# Для текущей сессии:
$env:DOCUMENT_SEARCHER_OCR_LANG = 'rus+eng'

# Или permanently (system-wide):
[Environment]::SetEnvironmentVariable("DOCUMENT_SEARCHER_OCR_LANG", "rus+eng", "User")

3. После этого перезапустить скрипт:
powershell -ExecutionPolicy Bypass -File "C:\Users\максим\Documents\programming\projects\document-searcher\scripts\verify-ocr-env.ps1"

---

Для GTK (опционально, если хочешь собирать --features desktop на Windows):
# Установить MSYS2 с https://www.msys2.org/
# Затем в MSYS2 shell:
pacman -S mingw-w64-x86_64-gtk4 mingw-w64-x86_64-libadwaita mingw-w64-x86_64-pkgconf

# Добавить в PATH Windows:
# C:\msys64\mingw64\bin
