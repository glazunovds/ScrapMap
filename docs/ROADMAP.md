# Roadmap

Документ фиксирует порядок зависимостей. Чекбокс означает состояние
репозитория, а не состояние старого исследовательского прототипа.

## M0 — Репозиторий и контракты

- [x] Создать Tauri 2 scaffold.
- [x] Перенести renderer без CE/RPC.
- [x] Исключить реальные captures, игровые изображения, имена и локальные пути.
- [x] Зафиксировать архитектуру и порядок работ.
- [x] Перенести обезличенные fixtures и renderer tests.
- [x] Определить versioned DTO: layout, telemetry, world identity, marker,
      fog delta и route.
- [x] Ввести интерфейсы `MapDataSource` и `SyncClient`.
- [x] Добавить CI для TypeScript/Rust/tests и secret/path scan.

Критерий: чистый clone собирается и открывает demo без локальных зависимостей
от старого прототипа.

## M1 — Tauri overlay PoC

- [x] Прозрачное undecorated always-on-top окно.
- [x] Click-through mini mode.
- [x] Интерактивная full map в том же окне.
- [x] Глобальные shortcut для mode и visibility.
- [x] Проверить focus, click-through и прозрачность в живом Windows-сеансе.
- [x] Win32 tracker окна Scrap Mechanic.
- [x] Следовать за client rect при move/resize и DPI change.
- [x] Скрывать overlay при minimize или потере foreground.
- [x] Перенести временный JSON watcher/normalizer внутрь Rust.
- [ ] Tray: status, open full map, diagnostics, exit.
- [x] Проверить, что Exit не оставляет процессов и портов.

Критерий: один debug/release EXE показывает текущую карту поверх оконной игры,
не крадёт мышь и сохраняет состояние при mini/full.

Это первая удобная точка остановки. Источник данных ещё может быть
диагностическим.

## M2 — Автоматический world/server profile и SQLite

Must have.

- [x] Канонический versioned `worldFingerprint` из layout.
- [x] Устойчивый `serverIdentity` для peer-hosted мира из PID-bound игрового
      лога; dedicated server пока не подтверждён.
- [x] `profileKey = party/server identity + world fingerprint`.
- [x] Автоматическое переключение `A → B → A`.
- [x] SQLite migrations и repository API в Rust.
- [x] Отдельные settings/fog/markers на профиль.
- [x] Команды хранения active route и session-bound breadcrumb trail.
- [x] PoC session/sequence gate: блокировать телеметрию во время активации и
      отклонять повторный или меньший `sequence` активной сессии.
- [x] Read-only fallback и ручное разделение профилей, если server ID
      неизвестен.

Критерий: возврат в известный мир автоматически восстанавливает только его
settings, fog, markers, активный route и ограниченный recent trail;
неразличимый мир остаётся read-only до явного выбора нового или известного
manual-профиля.

Контракт M2 использует SHA-256 и префиксы `smwf1_`/`smp1_` для канонического
layout и `profileKey`.
Transient lobby/session ID не считается устойчивым server ID. Все записи
проверяются по активной тройке `profileKey + worldFingerprint + sessionId`, а
локальная база располагается в
`%LOCALAPPDATA%\ScrapMap\scrapmap.sqlite3`. Storage vertical slice M2
завершён. Telemetry gate опирается на положительный возрастающий producer
`sequence` и обновление connection observation; полный lifecycle
join/death/rejoin и собственный producer epoch остаются M3.
Peer-hosted identity строится из PID-согласованного `game-*.log`; сырой
SteamID64 немедленно заменяется domain-separated SHA-256 и не покидает Rust.
Смена подключения подтверждается новым opaque connection observation и
согласуется с live host/non-host телеметрией; повторное старое наблюдение
переводит профиль в quarantine.
Любая неоднозначность включает fingerprint-only quarantine, который Rust
запрещает изменять.
Старые локальные fog/markers/filters Tauri PoC импортируются один раз и только
после успешной записи в SQLite; неизвестные remote-профили не импортируются
автоматически.

## M3 — CE-free read-only bridge

- [ ] Ранний console spike: terrain `0x0B` и telemetry без CE.
- [ ] Named pipe или shared-memory ring buffer.
- [x] Минимальный native transport открывает PID только для read/query,
      определяет main module и bounded `ReadProcessMemory` без write/hook API.
- [x] Read-only SHA-256 probe executable активного игрового PID без передачи
      локального пути в WebView.
- [x] Exact-SHA known-build allowlist с отдельным candidate layout id.
- [ ] Signature/prologue verification конкретного telemetry layout.
- [x] Feed-level compatibility gate: `unsupported` не публикует snapshot и
      не меняет игроков/fog; game build probe и hook guard остаются впереди.
- [ ] Join/death/rejoin/world switch lifecycle.
- [ ] Безопасный no-op при исчезновении ScrapMap host.
- [ ] Diagnostic fixtures и regression tests.

Критерий: на поддерживаемой версии non-host получает layout и игроков без
Cheat Engine; несовместимая версия завершается безопасным отказом.

Это второй решающий checkpoint: самостоятельный клиентский продукт.

## M4 — Настоящий tile atlas

Must have.

- [x] Локальный indexer UUID → relative PNG с размерами, SHA-256 и
      content fingerprint.
- [x] Native Tauri loader из `%LOCALAPPDATA%\ScrapMap\atlas` с ленивой выдачей
      PNG и безопасным fallback без атласа.
- [ ] Привязка индекса к отдельному game build fingerprint.
- [ ] Spike трёх способов сборки: isometric, rectified, large-zoom overlay.
- [ ] Rotation/alignment/road seam tests.
- [ ] Локальный cache только для UUID активного мира.
- [ ] Missing/corrupt preview fallback на schematic renderer.
- [ ] Инвалидация cache после обновления игры.
- [ ] Benchmark Canvas; WebGL только если измерения требуют.

Исходная проверка: 449/449 UUID тестового мира имеют PNG; всего найдено 807
preview. Игровые PNG не коммитятся и не распространяются. Indexer запускается
через `tools/tile-atlas/index.mjs`; для portable EXE его вывод размещается в
`%LOCALAPPDATA%\ScrapMap\atlas`, а репозиторный `runtime/` остаётся
игнорируемым исследовательским cache.

## M5 — POI catalog и поиск

Must have.

- [x] Stable POI ID и grouping многоклеточных объектов в клиентском каталоге.
- [ ] Категория/подкатегория.
- [ ] RU/EN display name, aliases и tags для полного игрового каталога.
- [ ] Нормальные icons и fallback.
- [x] Поиск по имени, alias, категории и координатам для загруженного layout.
- [ ] Выбор результата центрирует карту и назначает цель.
- [ ] Collision/clustering на малом масштабе.

Текущий M5 spike строит каталог на стороне Canvas из `poiId`/`groupId`, добавляет
локализованный типовой fallback, aliases и поиск с центрированием/подсветкой.
Критерий тестового мира пока не закрыт: все 139 POI должны иметь один стабильный
search result, категорию, имя и иконку.

## M6 — Локальная навигация

Must have.

### Метки

- [ ] UUID, точные world coordinates и cell.
- [ ] Label, color, icon/category.
- [ ] Create/edit/delete.
- [ ] Импорт/экспорт и хранение по world profile.

Отдельный флаг «посещено» не планируется: лут и противники могут respawn.

### Цель и направление

- [ ] POI, marker или произвольная точка как target.
- [ ] Прямая дистанция, bearing и cardinal direction.
- [ ] Стрелка цели относительно heading игрока.

### Breadcrumbs

- [ ] Sampling по времени и расстоянию.
- [ ] Разрыв при teleport/death/world change.
- [ ] Упрощение и ограничение старого следа.
- [ ] Очистка пользователем.

### Routes

- [ ] Road graph из взаимно согласованных road flags.
- [ ] Snap start/end к достижимой дороге.
- [ ] A* и понятный fallback на прямое направление.
- [ ] Road distance и direct distance.
- [ ] Re-route после отклонения.

Критерий: marker/POI становится целью, расстояние обновляется при движении,
маршрут не проходит через несовместимые road edges, а teleport не рисует
линию через весь мир.

Это третий checkpoint: полноценная ежедневная локальная карта.

## M7 — Cloudflare sync

Must have: общие метки и общий туман от обоих игроков.

- [ ] Local `SyncClient` + offline outbox.
- [ ] Один Worker + SQLite-backed Durable Object на room/world.
- [ ] Отдельный Bearer token для каждого игрока.
- [ ] World resolve по versioned fingerprint.
- [ ] Fog grow-only union.
- [ ] Shared marker upsert/delete с version и tombstone.
- [ ] Revision cursor + редкий incremental pull; WebSocket только если он
      действительно понадобится для presence.
- [ ] Idempotency keys и retry.
- [ ] Token revoke/rotation и одноразовые invites.
- [ ] Wrangler deploy, bindings и минимальная инструкция восстановления.

Критерий с двумя клиентами:

- fog A и B объединяется, даже когда игроки далеко друг от друга;
- оба создают и удаляют общие метки;
- offline операции доходят после reconnect и не дублируются;
- другой room/token не видит данные;
- eviction/redeploy Worker сохраняет состояние Durable Object;
- токены отсутствуют в логах и diagnostic bundle.

Это четвёртый checkpoint: cooperative alpha.

## M8 — Diagnostics и portable release

Must have.

- [ ] Явные состояния game missing/waiting/active/stale/unsupported/sync offline.
- [ ] Rotating structured logs.
- [ ] Redacted diagnostic bundle.
- [ ] Signed compatibility manifest для сборок игры.
- [ ] DB backup перед migration и recovery flow.
- [ ] Воспроизводимый portable EXE и опубликованный SHA-256 checksum.
- [ ] Clean exit без оставшихся процессов, портов или служб.
- [ ] Release checklist и проверка запуска с системным WebView2.

Критерий: изменение проверяемого пролога блокирует bridge, corrupt config/DB
восстанавливается из backup, logs ограничены, а portable EXE не требует
Node/Python/Rust/C++ toolchains. WebView2 Runtime является единственной
системной runtime-зависимостью; installer и встроенный updater не входят в
поставку.

## Вне scope первого релиза

- Workshop/world mod, требующий участия хоста.
- Изменение inventory, crafting или игровые RPC.
- DirectX hook для exclusive fullscreen.
- Распространение игровых PNG.
- E2E encryption, публичные аккаунты и большой multi-party backend.
