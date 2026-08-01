# Архитектура ScrapMap

## Цель

Один переносимый Windows EXE с двумя состояниями одного окна:

1. прозрачная click-through мини-карта поверх оконной или borderless игры;
2. интерактивная полная карта по глобальной клавише.

Приложение local-first: карта, туман, маршруты и метки продолжают работать
офлайн. VPS добавляет только совместную синхронизацию.

## Решения

### Tauri 2, Vite и TypeScript

Tauri подходит для прозрачного WebView2-окна, глобальных клавиш, Win32
интеграции, IPC, упаковки и будущего native bridge.

Next.js не используется. В Tauri он всё равно должен собираться как static
export; SSR, server actions и API routes не работают внутри desktop host.
Vite даёт тот же статический bundle с меньшим количеством соглашений.

Текущий PoC сохраняет существующий vanilla renderer, чтобы сначала проверить
рисковые свойства WebView2. Целевое состояние frontend:

- TypeScript для domain contracts и Canvas engine;
- React только для панелей, поиска, настроек и редакторов;
- imperative Canvas renderer не зависит от React lifecycle.

### Сохраняем Canvas 2D

Мир является конечной негеографической сеткой с поворотами, дорогами,
изометрическими изображениями, собственным туманом и динамическими игроками.
Leaflet и MapLibre не устраняют custom renderer, а добавляют второй camera
model.

Текущий renderer уже использует viewport culling и offscreen static cache.
PixiJS/WebGL рассматривается только после benchmark настоящего tile atlas.

### Один WebView

Mini и full map — состояния одного окна. Это сохраняет camera, filters и
frontend state без копирования между окнами.

```text
Mini
  transparent + always-on-top + ignore cursor events

Full
  размер клиентской области игры + interactive + focused
```

Оконный tracker уже находит top-level HWND процесса `ScrapMechanic.exe`,
получает client rect в физических screen pixels и опрашивает
move/resize/DPI/minimize/foreground раз в 200 мс. Имя image проверяется через
`PROCESS_QUERY_LIMITED_INFORMATION`; память процесса не открывается.

`userVisible` и фактическая видимость разделены. Поэтому автоматическое
скрытие при Alt-Tab не отменяет `Ctrl+Shift+H`. В full-режиме foreground окна
самой карты также считается активным, иначе интерактивная карта скрылась бы
сразу после получения фокуса. Изменение geometry, visibility, focusability и
click-through сериализовано одним transition lock.

При отсутствии игры discovery использует секундный backoff. Кэшированный HWND
переиспользуется только пока он видим, не свёрнут и имеет пригодную client
area; splash/устаревшее окно запускает новый поиск лучшего кандидата.

### Источники данных

Renderer не знает, откуда пришли данные:

```text
MapDataSource
  loadLayout()
  subscribeTelemetry()
  subscribeStatus()

DemoDataSource        — безопасные fixtures
DiagnosticDataSource  — встроенный bounded JSON reader без TCP-порта
NativeBridgeSource    — конечный read-only bridge
```

Layout передаётся один раз при смене мира. Для применения телеметрии текущий
PoC требует положительный JS-safe `sequence`: WebView просит Rust принять его
для точной активной profile session, а повторный или меньший номер
игнорируется. Пока переключается профиль, кадры не применяются. При смене
opaque connection observation тот же layout активируется заново с новым
`sessionId`. Это in-memory session gate, а не подписанный producer epoch.
Многомегабайтный layout не отправляется на каждом кадре.

Текущий diagnostic reader работает в Rust, опрашивает один файл раз в 100 мс,
ограничивает его размер 1 MiB, проверяет стабильность записи и валидирует
игроков/координаты до Tauri event. WebView не получает путь к файлу. При
partial write остаётся последний корректный snapshot.

Feed-level compatibility gate уже поддерживает `source.compatibility`:
`unsupported` от будущего native provider не публикуется в WebView и не меняет
последние координаты или fog. Это не заменяет build fingerprint/signature
проверки, а фиксирует fail-closed границу до появления реального provider.

### Native bridge

Финальный bridge выполняет только два read-only действия:

- копирует уже полученный клиентом terrain packet `0x0B`;
- читает позиции, направления и имена реплицируемых игроков.

Он не содержит inventory/freecraft/RPC функций и не отправляет команды
игровому серверу. Перед установкой hook обязательны build fingerprint,
signature/prologue checks и fail-closed поведение.

Команда `game_build_probe` уже вычисляет SHA-256 executable процесса,
найденного Win32 window tracker, и возвращает только hash, размер,
`smgb1_…` compatibility ID и (только при точном совпадении allowlist) native
candidate layout id. Путь к установленной игре не передаётся в WebView.
Известный SHA получает `recognized`, а неизвестный — `unsupported`; оба
состояния остаются закрыты для reader до prologue check.

Команда `native_transport_probe` проверяет независимую нижнюю ступень: процесс
доступен с правами `PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ`, main
module является PE64, а чтение ограничено его размером. В модуле намеренно нет
операций записи, выделения памяти, remote thread или hook. Состояние `ready`
означает только исправный транспорт, а не найденный telemetry layout.

### Локальное хранение

Desktop host использует SQLite для world profiles; прежний `localStorage`
остаётся только fallback для browser/demo режима. Базой владеет Rust; WebView
получает узкие команды активации профиля, объединения тумана, замены локальных
меток, сохранения настроек, установки/очистки активного маршрута и пакетной
записи breadcrumbs, а не произвольный SQL. Profile snapshot содержит
`activeRoute` и `recentTrail`; recent trail сообщает полное число точек, но
возвращает не более 4096 последних точек и флаг `truncated`.
Файл базы располагается в
`%LOCALAPPDATA%\ScrapMap\scrapmap.sqlite3`: расположение EXE не влияет на
профили, а удаление portable-файла не удаляет пользовательские данные.
При первом открытии локального профиля bridge проверяет прежние ключи
`localStorage` от Tauri PoC и переносит fog/markers/filters только если
соответствующая SQLite-сущность ещё пуста. Маркер миграции записывается лишь
после успешных native-записей. Для fingerprint-only профиля чужого сервера
такой импорт намеренно не выполняется, чтобы не смешать данные разных миров с
одинаковым старым `worldId`.

Идентификаторы M2 вычисляются детерминированно:

- `worldFingerprint` имеет вид `smwf1_<64 lowercase hex>` и является SHA-256
  от canonical JSON с domain `scrapmap-world-v1`, cell size, bounds и
  ячейками. UUID приводятся к lowercase, ячейки сортируются по
  `(x, y, tileUuid)`; для каждой сохраняются только `x/y`, UUID, rotation,
  offsets и stable flags. Локальные пути, исходный `worldId`, отображаемые
  POI-названия, game mode, timestamps и номер игровой сессии исключены. Game
  mode хранится как изменяемая метаинформация профиля и поэтому его позднее
  распознавание не создаёт новый мир;
- `profileKey` имеет вид `smp1_<64 lowercase hex>` и является SHA-256 от JSON
  массива `["scrapmap-profile-v1", scopeKind, scopeId, worldFingerprint]`.
  Scope выбирается как `local`, устойчивый `server` или явный `fallback`;
- transient Steam lobby/session ID не считается `serverIdentity`. Если
  устойчивого ID нет, профиль помечается как требующий ручного разделения.

Для friend-hosted мира native host сначала пытается получить устойчивый
`serverIdentity` без доступа к памяти игры:

1. Win32 tracker сообщает PID найденного `ScrapMechanic.exe`;
2. bounded reader выбирает последний файл точного вида
   `Logs/game-YYYYMMDD-HHMMSS.log`, проверяет PID в его заголовке и последнюю
   завершённую строку `Connecting to ...`;
3. `Connecting to self` означает локальный мир, а SteamID64 хоста немедленно
   превращается в
   `steam-sha256:SHA-256("scrapmap-peer-host-v1\0" + steamId64)`;
4. конкретное connection-событие получает отдельный обезличенный
   `connection-sha256:...`; identity принимается только после повторного
   наблюдения, совпадения с live `isHost` и, при переподключении, смены этого
   observation;
5. несовпадение PID, новый формат, partial write, файл больше 4 MiB или любая
   ошибка дают `unknown`.

Сырой SteamID64, путь и строки лога не покидают Rust. Connection handle,
runtime world index и transient lobby ID не используются. Dedicated server
пока остаётся неподтверждённым сценарием.

Если identity остаётся `unknown`, автоматически выбирать один из ранее
созданных manual-профилей нельзя: два сервера с одинаковым terrain layout
неразличимы. Fingerprint-only профиль поэтому является read-only quarantine.
UI предлагает именованные manual-профили только этого fingerprint; выбор или
создание повторно активирует профиль с новым `sessionId` и opaque
`manual:<UUID v4>`. Другие fallback ID native storage отклоняет.
Новые клетки тумана до выбора остаются только в памяти и переносятся в
выбранный профиль; staging markers/settings не копируются. При каждом новом
неразличимом подключении выбор нужно подтвердить снова.

Каждое изменение профиля несёт `profileKey`, `worldFingerprint` и `sessionId`.
Rust принимает запись только для текущего активного контекста. Это session
gate не позволяет запоздалой операции старого подключения раскрыть туман или
изменить данные уже выбранного мира. Второй write gate отклоняет все persistent
mutation для quarantine-профиля до ручного разрешения.
Проверка telemetry sequence использует тот же точный context, но остаётся
доступной в quarantine, поскольку сама по себе ничего не сохраняет. Она
упорядочивает кадры внутри выбранной активации, но не доказывает их
producer-сессию.

Текущая migration создаёт:

- world profiles и layouts;
- settings;
- fog cells;
- marker records;
- routes, trails и breadcrumb points;
- последнее активное состояние приложения.

В текущем vertical slice repository API реализован для профилей/layout,
settings, fog cells, локальных markers, одного активного route на профиль и
session-bound trails/breadcrumbs. Route проверяется против мира и его ячеек;
trail batches должны быть последовательными, а одинаковый повтор принимается
идемпотентно. Устаревшая session и конфликтующий повтор отклоняются. Это только
слой хранения: A*, отрисовка маршрута, sampling и упрощение breadcrumbs
остаются M6. Sync cursor и offline outbox появятся вместе с VPS sync.

Tile atlas хранится файлами в cache directory, а не BLOB в SQLite.

### Tile atlas

В установленной игре найдено 807 PNG-preview тайлов размером `220×150` и 805
уникальных UUID; два UUID имеют варианты в разных подкаталогах. Скрипт
`tools/tile-atlas/index.mjs` строит локальный versioned manifest с относительным
путём, размером, SHA-256 каждого файла и content fingerprint набора. Сейчас
это fingerprint содержимого atlas, а не отдельный fingerprint executable
build.

Приложение строит cache локально из установленной игры и не распространяет
игровые изображения. Для portable EXE manifest и PNG-копии находятся в
`%LOCALAPPDATA%\ScrapMap\atlas`; репозиторный `runtime/` остаётся только
локальным исследовательским cache и игнорируется Git. Native bridge командой
`atlas_manifest` получает manifest, а `atlas_preview` отдаёт отдельный PNG как
data URL. Исследовательские задачи: маскирование фона,
изометрическая проекция, rotation, seams и z-order. Canvas renderer принимает
manifest через `window.SMMinimap.setTileAtlas(manifest, options)`: `baseUrl` или
`resolveSource(entry)` дают WebView URL изображения, а при ошибке загрузки
включается прежний schematic fallback.

### POI catalog spike

`map-core.js` строит transient catalog из нормализованного layout: `poiId` и
`groupId` образуют устойчивый ключ, а несколько клеток одного объекта
объединяются. Для неизвестного типа используется координатный fallback. В UI
поиск принимает имя, английское имя, aliases, категорию и координаты; выбор
центрирует карту и подсвечивает группу. Полный каталог всех 139 игровых POI,
проверенные иконки и назначение маршрутизируемой цели остаются M5/M6.

### Разделение локального и общего состояния

```text
ScrapMap.exe
├─ local world/profile database
├─ renderer + overlay
├─ game data source
└─ SyncClient (optional)
       │ HTTPS
       ▼
scrapmap-sync on VPS
├─ parties/members/tokens
├─ world identities
├─ fog union
└─ shared markers
```

На пользовательском ПК нет Node/Python server. Для локального режима не
открывается TCP port. VPS sync включается явно.

## Предлагаемая структура

Пока репозиторий остаётся простым, без преждевременного monorepo:

```text
src/
  domain/
  data-sources/
  map-engine/
  ui/
src-tauri/src/
  overlay/
  game_window/
  telemetry/
  storage/
  sync/
tools/
  tile-atlas/
docs/
public/map/          # временный renderer PoC
```

Сервис синхронизации добавляется отдельным Rust package только перед VPS
этапом.

## Упаковка

Цель — один portable `scrapmap.exe`, без MSI/NSIS, службы и встроенного
updater. Пользователю не нужны build toolchains. Приложение использует
системный Evergreen WebView2; если runtime отсутствует, пользователь
устанавливает официальный WebView2 Runtime отдельно.

Runtime-данные находятся в `%LOCALAPPDATA%\ScrapMap`, поэтому обновление EXE не
трогает профили. Release-процесс должен публиковать checksum и совместимый
versioned manifest; автоматическая установка обновлений не является частью
продукта.
