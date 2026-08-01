# Cloudflare sync — направление v1

Cloudflare не рендерит карту и не хранит game assets. Один Worker маршрутизирует
запросы в SQLite-backed Durable Object, выбранный по opaque room id. Такой
object сериализует изменения общего тумана и меток и переживает eviction или
redeploy без отдельного VPS-процесса.

```text
ScrapMap A ─┐
            ├─ HTTPS Worker ─ Durable Object(room) ─ SQLite storage
ScrapMap B ─┘
```

Для PoC не нужны D1, KV, R2, WebSocket и отдельный backend. Всё состояние одной
группы живёт в одном Durable Object; следующая группа получает другой object.

## Нагрузка

Клиент отправляет fog delta пакетами раз в несколько секунд и marker mutation
сразу. Snapshot загружается при подключении, затем клиент запрашивает изменения
по revision cursor с умеренным интервалом. Два постоянно активных клиента не
должны делать частый polling: 5 секунд достаточно для меток и тумана.

Hibernation WebSocket оставляем опцией для будущего live presence. Для первого
этапа HTTP проще, а координаты игроков не требуется сохранять на сервере.

## Доступ

У room есть случайный идентификатор и отдельный bearer token для каждого
устройства. Токен хранится Tauri-клиентом, передаётся в `Authorization`, не
помещается в query string и не выводится в diagnostic logs. Оба игрока имеют
право добавлять и удалять общие метки.

## API v1

```http
GET    /healthz
GET    /v1/rooms/{room}/snapshot
GET    /v1/rooms/{room}/changes?after=<revision>
PUT    /v1/rooms/{room}/fog
PUT    /v1/rooms/{room}/markers/{marker-id}
DELETE /v1/rooms/{room}/markers/{marker-id}
```

Каждая mutation содержит `Idempotency-Key`. Ответ mutation возвращает новую
server revision.

### Fog

Клиент отправляет только новые cells вокруг собственного игрока. Fog является
grow-only set: Durable Object выполняет monotonic union. Координаты игрока и
breadcrumbs на Cloudflare не отправляются.

### Markers

- marker имеет стабильный UUID;
- update/delete содержит ожидаемую version;
- конфликт возвращает `409` и текущую запись;
- delete создаёт tombstone;
- server revision определяет порядок без доверия к часам клиента.

## Клиент

Локальная SQLite содержит outbox. Offline fog и marker mutations повторяются
после reconnect, поэтому операции должны быть идемпотентными. Profile/world
fingerprint остаётся локальным и сопоставляется с room только после явного join.

## Границы PoC

- bounded JSON body и fog batch;
- координаты проверяются по world bounds;
- `Cache-Control: no-store` для приватного состояния;
- неизвестный token получает одинаковый `401` без раскрытия room;
- никакой панели администратора, OAuth, D1 или сложной ролевой модели;
- проверка восстановления состояния после локального Miniflare eviction и
  Cloudflare redeploy.

Live presence можно добавить отдельным ephemeral WebSocket после общей
синхронизации fog/markers. Он не должен блокировать первый cooperative
checkpoint.
