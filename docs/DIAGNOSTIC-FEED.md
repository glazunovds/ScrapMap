# Диагностический JSON-канал

Это временный read-only вход для разработки overlay до появления native
bridge. ScrapMap сам читает файл в Rust: Node/Python server и localhost-порт
не нужны.

## Выбор файла

Приоритет источников:

1. `--telemetry-file PATH` или `--telemetry-file=PATH`;
2. `SCRAPMAP_TELEMETRY_FILE`;
3. `%LOCALAPPDATA%\ScrapMap\diagnostic\telemetry.json`.

Путь остаётся в native host и не передаётся WebView. Diagnostic events также
не содержат путь.

## Минимальный payload

```json
{
  "schemaVersion": 2,
  "worldId": "synthetic-world-v1",
  "timestamp": "2026-01-01T00:00:00.000Z",
  "sequence": 1,
  "localPlayerId": "player-a",
  "staleAfterMs": 2000,
  "source": {
    "type": "diagnostic-fixture",
    "enumeration": "getAllPlayers",
    "isHost": false,
    "compatibility": "supported"
  },
  "player": {
    "id": "player-a",
    "name": "Player A",
    "x": 54,
    "y": -18,
    "z": 12,
    "heading": 34
  },
  "players": [
    {
      "id": "player-a",
      "name": "Player A",
      "isLocal": true,
      "active": true,
      "hasCharacter": true,
      "sameWorld": true,
      "x": 54,
      "y": -18,
      "z": 12,
      "heading": 34
    },
    {
      "id": "player-b",
      "name": "Player B",
      "isLocal": false,
      "active": true,
      "hasCharacter": true,
      "sameWorld": true,
      "x": -46,
      "y": 70,
      "z": 12,
      "heading": 215
    }
  ]
}
```

Если `worldId` указан, он должен совпасть с уже загруженным layout. Snapshot
другого мира отклоняется до изменения позиции или тумана. Snapshot публикуется
только при наличии одного пригодного локального игрока:
`active`, `hasCharacter`, `sameWorld` и конечные `x/y/z`.

Для применения в overlay `sequence` обязателен: это положительное целое число,
которое возрастает в пределах текущей активации профиля. Кадр без него, с
нулём, повторным или меньшим значением не двигает игроков и не раскрывает
туман. После перезапуска writer должен продолжить счётчик либо дождаться новой
активации профиля.

## Безопасность чтения

- максимальный файл: 1 MiB;
- максимум 64 игрока;
- имена, ID, координаты, векторы и счётчики ограничены;
- `heading` нормализуется в диапазон 0–360;
- размер и metadata проверяются до и после чтения;
- одинаковое содержимое не публикуется повторно;
- partial/invalid write возвращает состояние `waiting`, `stale` или `invalid`,
  но сохраняет последний корректный snapshot.

Свежесть считается в native host по монотонному времени получения корректного
содержимого и `staleAfterMs`. Поле `timestamp` остаётся информационным и не
может удержать поток в состоянии `active`, если writer остановился.

Writer по возможности должен сохранять новый JSON во временный файл, а затем
атомарно переименовывать его в `telemetry.json`. Reader всё равно безопасно
обрабатывает обычную частичную перезапись.

Поле `source.compatibility` зарезервировано для будущего native read-only
provider. Значение `unsupported` переводит native host в состояние
`unsupported`: snapshot не публикуется, последний корректный snapshot не
подменяется, игроки и fog не обновляются. Значения `supported` и `unknown`
сохраняют текущую совместимость с диагностическим feed.

## Не является продуктовым bridge

Этот канал не извлекает данные из Scrap Mechanic. На текущем этапе JSON может
создавать внешний исследовательский инструмент. Конечный native bridge будет
отдельным read-only источником с проверкой версии игры и безопасным отказом
при неизвестной сборке.
