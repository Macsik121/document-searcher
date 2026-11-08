\# Document Searcher



\*\*Document Searcher\*\* — высокопроизводительное настольное (GUI) и консольное (CLI) приложение на языке Rust для глубокого контекстного поиска текста внутри локальных документов \*\*`.docx`\*\* и \*\*`.pdf`\*\* на платформах \*\*Windows 10/11\*\*, \*\*ALT Linux\*\* и \*\*Ubuntu\*\*.



\---



\## Основные возможности



\* \*\*Фокус на документах `.docx` и `.pdf`:\*\*

&#x20; \* По умолчанию сканер индексирует только файлы `.docx` и `.pdf` (регистронезависимо), игнорируя исходный код, временные и скрытые файлы (начинающиеся с `.`), символические ссылки и файлы размером более \*\*50 MiB\*\*.

&#x20; \* При необходимости список форматов можно расширить через параметр `--ext` (например, `--ext docx,pdf,txt,md`).

\* \*\*Трёхуровневый конвейер извлечения текста из PDF (`3-Tier PDF Extractor`):\*\*

&#x20; 1. \*\*Постраничный декодер `lopdf` с поддержкой кириллицы:\*\* напрямую разбирает шрифтовые таблицы `/Differences` (глифы `/uni0410`..`/uni044F`, `/afii10017`..`/afii10097`, `.cap`), карты символов `/ToUnicode` (`bfchar` / `bfrange`), `UTF-16BE` и кодировку `CP1251`.

&#x20; 2. \*\*Изолированный парсер `pdf-extract`:\*\* выполняется внутри защитного контура `std::panic::catch\_unwind`, благодаря чему сложные или повреждённые PDF-файлы никогда не прерывают общий поиск.

&#x20; 3. \*\*Интеллектуальный OCR-фоллбэк для сканов:\*\* если PDF не содержит текстового слоя, автоматически запускается распознавание через `pdftoppm` + `tesseract` (`eng+rus` с откатом на `eng`), а на Windows 10/11 при отсутствии внешних утилит используется встроенный системный движок \*\*Windows WinRT (`Windows.Data.Pdf` + `Windows.Media.Ocr`)\*\*.

\* \*\*Мгновенный повторный поиск (`In-Memory TEXT\_CACHE`):\*\*

&#x20; \* Извлечённый текст каждого файла кэшируется в оперативной памяти по ключу `(путь, размер, время модификации)`. Повторные поисковые запросы по проиндексированной папке выполняются за миллисекунды без повторного чтения диска.

\* \*\*Многопоточность на всех ядрах CPU:\*\*

&#x20; \* Извлечение текста и поиск выполняются параллельно в пуле `tokio::task::JoinSet`, ограниченном семафором по числу ядер процессора (`available\_parallelism`, от 2 до 8 потоков).

&#x20; \* Защита от «keystroke bomb»: задержка ввода (\*\*debounce 300 мс\*\*), мгновенная отмена устаревшего поиска через `CancellationToken` и строгая сериализация команд через монитор `Controller`.

\* \*\*Unicode-поиск с контекстом (±20 символов):\*\*

&#x20; \* Регистронезависимый поиск с сохранением точных байтовых границ символов UTF-8.

&#x20; \* Для каждого совпадения формируется сниппет: \*\*до 20 символов до\*\* + \*\*точная найденная подстрока (выделяется маркером)\*\* + \*\*до 20 символов после\*\*.

\* \*\*Два режима работы в одном исполняемом файле (GUI + CLI):\*\*

&#x20; \* \*\*Графический интерфейс (GUI):\*\* разделение экрана 50/50, список найденных файлов слева, строка поиска с индикацией прогресса (спиннер / галочка успеха / ошибка), карточки результатов с подсветкой совпадений, кумулятивная пагинация по \*\*100 результатов\*\* (`Show more`), выбор папки через системный диалог и открытие документов по клику в программе по умолчанию.

&#x20; \* \*\*Консольный интерфейс (CLI):\*\* быстрые команды `scan` и `search` для работы в терминале и скриптах.



\---



\## Быстрый старт и развёртывание (Windows 10/11)



\### 1. Предварительные требования

\* Установленный тулчейн \*\*Rust\*\* (`rustup`, версия `1.78.0` закреплена в `rust-toolchain.toml`).

\* Никаких дополнительных C/C++ библиотек на Windows устанавливать не требуется.



\### 2. Настройка корневого Workspace (выполняется один раз)

Если вы запускаете команды из корня репозитория (над папкой `src`), убедитесь, что в корне лежит файл `Cargo.toml` со следующим содержимым:



```toml

\[workspace]

members = \["src"]

default-members = \["src"]

resolver = "2"

```



\### 3. Глобальная установка для запуска из любой папки

Выполните в PowerShell из корня репозитория:



```powershell

cargo install --path .\\src --features windows-desktop --force

```



Эта команда скомпилирует релизный бинарник `document-searcher.exe` и установит его в `%USERPROFILE%\\.cargo\\bin`, после чего приложение можно вызывать из \*\*любой\*\* директории на компьютере.



\### 4. Сборка и запуск через Cargo (без глобальной установки)



```powershell

\# Сборка релизной версии

cargo build --release --features windows-desktop



\# Запуск графического интерфейса (GUI)

cargo run --release --features windows-desktop



\# Запуск GUI сразу для конкретной папки

cargo run --release --features windows-desktop -- "C:\\Users\\Имя\\Documents\\Отчёты"



\# Запуск консольного сканирования и поиска (CLI)

cargo run --no-default-features -- scan "C:\\Users\\Имя\\Documents\\Отчёты"

cargo run --no-default-features -- search "C:\\Users\\Имя\\Documents\\Отчёты" "искомая строка"

```



\---



\## Использование установленного приложения



После выполнения `cargo install` откройте терминал в любой нужной папке с документами:



```powershell

\# 1. Открыть графическое окно поиска для текущей папки

document-searcher .



\# 2. Открыть графическое окно поиска для указанной папки

document-searcher "D:\\Документы\\Договоры"



\# 3. Открыть графическое окно с последней сохранённой папкой

document-searcher



\# 4. Просканировать папку в консоли (по умолчанию только .docx и .pdf)

document-searcher scan .



\# 5. Найти подстроку во всех .docx и .pdf файлах папки в консоли

document-searcher search . "финансовый отчёт"



\# 6. Запустить поиск с пользовательским списком расширений (--ext)

document-searcher search . "config" --ext docx,pdf,txt,md

```



\---



\## Развёртывание на ALT Linux и Ubuntu



\### 1. Установка системных зависимостей (для OCR и системных диалогов)



\* \*\*На ALT Linux (`apt-rpm`):\*\*

&#x20; ```bash

&#x20; sudo apt-get update

&#x20; sudo apt-get install -y gcc make pkgconfig tesseract tesseract-langpack-ru tesseract-langpack-en poppler-tools zenity xdg-utils

&#x20; ```

\* \*\*На Ubuntu / Debian (`apt`):\*\*

&#x20; ```bash

&#x20; sudo apt update

&#x20; sudo apt install -y build-essential pkg-config tesseract-ocr tesseract-ocr-rus tesseract-ocr-eng poppler-utils zenity xdg-utils

&#x20; ```



\### 2. Установка Rust (если не установлен)

```bash

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y

source "$HOME/.cargo/env"

```



\### 3. Сборка и установка приложения

```bash

\# Сборка и глобальная установка в \~/.cargo/bin

cargo install --path ./src --force



\# Запуск в текущей директории

document-searcher .

```



\*(Примечание: для сборки классического варианта на базе нативных библиотек GTK4 / Libadwaita и упаковки во Flatpak на ALT Linux см. инструкции в \[`docs/ALT-LINUX-RUN.md`](docs/ALT-LINUX-RUN.md) и \[`src/BUILD\_INSTRUCTIONS.md`](src/BUILD\_INSTRUCTIONS.md)).\*



\---



\## Проверка качества кода и тестирование



Перед коммитом или релизом выполните стандартный набор проверок из корня репозитория (или из папки `src/`):



```powershell

\# 1. Проверка форматирования кода

cargo fmt --check



\# 2. Запуск unit-тестов ядра (scanner, extractor, search, worker, state)

cargo test --lib --no-default-features



\# 3. Запуск unit-тестов десктопного контроллера и сериализации

cargo test --bins --features windows-desktop



\# 4. Статический анализ Clippy (без единого предупреждения)

cargo clippy --lib --tests --no-default-features -- -D warnings

cargo clippy --all-targets --features windows-desktop -- -D warnings

```



\---



\## Структура проекта и документация



```text

document-searcher/

├── Cargo.toml                         # Корневой Cargo Workspace

├── README.md                          # Основное руководство по проекту

├── docs/

│   ├── UNIFIED-ARCHITECTURE-SPEC.md   # Единая спецификация v2.0 (SRS + BPMN + UML + ERD)

│   ├── ALT-LINUX-RUN.md               # Руководство по запуску в ALT Linux VM

│   ├── architecture/                  # Диаграммы компонентов и потоков

│   └── specs/                         # Исходные требования к проекту

└── src/

&#x20;   ├── Cargo.toml                     # Манифест пакета document-searcher

&#x20;   ├── rust-toolchain.toml            # Закреплённая версия компилятора (1.78.0)

&#x20;   ├── build.rs                       # Скрипт сборки

&#x20;   ├── BUILD\_INSTRUCTIONS.md          # Подробные инструкции по сборке

&#x20;   └── src/

&#x20;       ├── lib.rs                     # Библиотечная точка входа ядра

&#x20;       ├── main.rs                    # Универсальная точка входа (GUI + CLI)

&#x20;       ├── app\_win.rs                 # Десктопный контроллер, HTTP-пул и встроенный UI

&#x20;       ├── app.rs                     # Опциональный GTK4/Libadwaita адаптер для Linux

&#x20;       └── model/

&#x20;           ├── actions.rs             # Протокол команд и событий (WorkerCommand / WorkerEvent)

&#x20;           ├── state.rs               # Состояние приложения (AppState) и пагинация

&#x20;           ├── scanner.rs             # Сканер директорий (.docx, .pdf, лимит 50 MiB)

&#x20;           ├── extractor.rs           # 3-уровневый экстрактор (lopdf + docx + OCR + TEXT\_CACHE)

&#x20;           ├── search.rs              # Регистронезависимый Unicode-поиск с контекстом ±20 симв.

&#x20;           └── worker.rs              # Многопоточный Tokio-воркер (Debounce 300ms, JoinSet)

```



\* \*\*\[Единая спецификация и архитектура (BPMN, UML, ERD)](docs/UNIFIED-ARCHITECTURE-SPEC.md)\*\*

\* \*\*\[Исходные требования](docs/specs/requirements-spec.md)\*\*

\* \*\*\[Запуск на ALT Linux](docs/ALT-LINUX-RUN.md)\*\*



