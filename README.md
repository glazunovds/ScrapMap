# ScrapMap

Локальная карта и оконный overlay для Scrap Mechanic.

Проект развивается как отдельное read-only приложение: он визуализирует
полученные клиентом данные мира, но не изменяет инвентарь, серверное состояние
или игровые RPC.

## Текущий checkpoint

В репозитории находится рабочий checkpoint Tauri 2 overlay:

- текущий проверенный Canvas renderer запускается внутри WebView2;
- компактное окно прозрачно, находится поверх остальных окон и пропускает
  клики;
- Win32 tracker находит `ScrapMechanic.exe`, привязывает окно к физической
  client area игры и учитывает DPI;
- mini автоматически скрывается при сворачивании игры или переключении на
  другое приложение и возвращается при возврате в игру;
- `Ctrl+Shift+M` переключает компактную и полную интерактивную карту;
- `Ctrl+Shift+H` скрывает или возвращает overlay, причём ручное скрытие не
  отменяется foreground-трекером;
- `Ctrl+Shift+Q` полностью завершает тестовое приложение;
- `Escape` возвращает полную карту в компактный режим;
- временный диагностический JSON читается непосредственно Rust-кодом без
  Node-сервера и localhost-порта;
- неполная или некорректная запись не роняет приложение и не затирает
  последнюю корректную позицию;
- при загрузке layout Rust вычисляет канонические `worldFingerprint` и
  `profileKey`, активирует изолированный SQLite-профиль и возвращает его
  туман, локальные метки, настройки, активный маршрут и ограниченный хвост
  последнего breadcrumb-трека;
- записи профиля защищены текущими `profileKey + worldFingerprint +
  sessionId`, поэтому отложенная операция предыдущего активного контекста
  отклоняется; те же проверки применяются к узким командам сохранения
  маршрута и пакетной записи breadcrumbs;
- для peer-hosted мира Rust сопоставляет PID игры с её текущим bounded
  `game-*.log`, превращает SteamID64 хоста в domain-separated SHA-256 и
  автоматически восстанавливает профиль этого сервера; WebView принимает
  identity только после согласования с live host/non-host телеметрией и
  обезличенным connection observation; при смене observation секундный
  монитор запускает новую активацию профиля с новым `sessionId`;
- диагностическая телеметрия применяется только при положительном
  возрастающем `sequence`: повторные и более старые кадры, а также кадры во
  время переключения профиля отбрасываются. Это практический gate для PoC, а
  не полный producer connection epoch;
- если безопасно распознать сервер нельзя, fingerprint-only профиль работает
  как read-only quarantine: туман, метки и настройки начинают сохраняться
  только после выбора или создания именованного профиля в компактном диалоге;
- прежние fog/marker/filter-данные Tauri PoC однократно переносятся из
  `localStorage` в SQLite после подтверждённых native-записей локального
  профиля; данные неизвестного удалённого сервера автоматически не смешиваются;
- в Git включено только синтетическое demo. Capture-файлы, имена игроков,
  локальные пути и игровые изображения намеренно исключены.

Трекер не читает память игры: он использует только обычные Win32-сведения об
окне и process image с правом `PROCESS_QUERY_LIMITED_INFORMATION`. Распознавание
peer-hosted сервера читает не более 4 MiB текущего игрового лога; сырой
SteamID64, путь и строки лога не передаются в WebView и не сохраняются в
SQLite. Диагностический JSON остаётся переходным источником до отдельного
CE-free read-only bridge.

## Разработка

Требования для разработчика:

- Node.js 20+;
- pnpm;
- Rust stable с target `x86_64-pc-windows-msvc`;
- Visual Studio Build Tools с C++ workload;
- WebView2 Runtime.

```powershell
pnpm install
pnpm tauri dev
```

Проверка frontend и Rust:

```powershell
pnpm build
pnpm test
cargo check --manifest-path .\src-tauri\Cargo.toml
cargo test --manifest-path .\src-tauri\Cargo.toml
```

Конечному пользователю Node, pnpm, Rust, Python, GCC или Visual Studio не
потребуются. Релиз поставляется одним переносимым `scrapmap.exe`; системный
Microsoft Edge WebView2 Runtime должен быть установлен в Windows.

## Portable release

Release-сборка намеренно не создаёт MSI/NSIS, не устанавливает службу и не
регистрирует updater:

```powershell
pnpm tauri build
```

Готовый файл находится в:

```text
src-tauri\target\release\scrapmap.exe
```

Сам EXE можно хранить в любом каталоге. Пользовательские данные не записываются
рядом с ним: диагностический канал и SQLite-профили мира располагаются в
`%LOCALAPPDATA%\ScrapMap`. Удаление EXE не удаляет эти данные автоматически.
На системе без WebView2 Runtime его нужно установить официальным средством
Microsoft отдельно.

## Диагностическая телеметрия

По умолчанию приложение следит за:

```text
%LOCALAPPDATA%\ScrapMap\diagnostic\telemetry.json
```

Другой файл можно выбрать только для текущего запуска:

```powershell
.\scrapmap.exe --telemetry-file "D:\temporary\telemetry.json"
```

Также поддерживается переменная `SCRAPMAP_TELEMETRY_FILE`; аргумент командной
строки имеет приоритет. Формат и ограничения описаны в
[диагностическом канале](docs/DIAGNOSTIC-FEED.md).

## Локальный tile atlas

В установленной игре можно построить локальный manifest настоящих terrain
preview-файлов:

```powershell
$gameRoot = "<путь к установленной Scrap Mechanic>"
$atlasRoot = Join-Path $env:LOCALAPPDATA "ScrapMap\atlas"
node tools/tile-atlas/index.mjs `
  --game-root $gameRoot `
  --output (Join-Path $atlasRoot "manifest.json") `
  --copy-to (Join-Path $atlasRoot "tiles")
```

Manifest содержит только относительные пути, размеры и SHA-256; каталог
`runtime/` игнорируется Git. В текущей установке найдено 807 preview-файлов
размером 220×150 и 805 уникальных tile UUID. Portable EXE автоматически
проверяет `%LOCALAPPDATA%\ScrapMap\atlas\manifest.json` и подгружает PNG по
мере появления тайлов на экране; отдельный сервер или установленная Node.js
для этого не нужны. Если каталог не создан, manifest повреждён или отдельный
PNG недоступен, остаётся текущий schematic fallback. Для браузерного режима
renderer по-прежнему можно подключить вручную через
`window.SMMinimap.setTileAtlas(manifest, options)`.

## Документация

- [Архитектура](docs/ARCHITECTURE.md)
- [Последовательность работ](docs/ROADMAP.md)
- [Синхронизация через Cloudflare Worker](docs/SYNC.md)
- [Диагностический JSON-канал](docs/DIAGNOSTIC-FEED.md)

В текущем renderer уже есть локальный POI search spike: поиск работает по имени,
английскому имени, aliases, категории и координатам загруженного layout; выбор
результата центрирует карту и подсвечивает все клетки сгруппированного POI.

## Наследие прототипа

`public/map` временно содержит dependency-free HTML/CSS/JS renderer из
исследовательского прототипа. После проверки поведения прозрачного WebView он
будет постепенно разделён на:

- framework-independent Canvas engine;
- versioned domain contracts;
- Vite + TypeScript UI;
- Tauri data sources.

Часть с Cheat Engine не переносится в продукт. До появления native read-only
bridge она может использоваться только как внешний диагностический источник
JSON во время разработки.

M3 build probe уже умеет read-only вычислять SHA-256 executable активного
процесса Scrap Mechanic без раскрытия пути и выбирает candidate native layout
только при точном совпадении локального allowlist. Это пока диагностика, а не
извлечение координат: `recognized` не станет `supported` до signature/prologue
checks.

Нижний native transport уже работает без CE: он открывает найденный игровой PID
только для query/read, проверяет PE64 main module и умеет bounded-чтение через
`ReadProcessMemory`. Адреса и содержимое памяти не передаются в WebView; до
обнаружения и проверки конкретного telemetry layout координаты не публикуются.

Хранение route/trail уже подключено как инфраструктура. Построение и
отрисовка маршрута, а также автоматический sampling breadcrumbs остаются
отдельным этапом навигации.
