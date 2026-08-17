---
tdr_id: "TDR-FS-2.3"
title: "Terminal routing и terminal-dialogue"
status: "принято"
implementation_stage: "текущее"
parent_tdr_id: "TDR-FS-2"
child_tdr_ids: []
updated: "2026-08-16"
---
# TDR-FS-2.3 — Terminal routing и `terminal-dialogue`

## Контекст TDR

- [TDR-FS-2](<TDR-FS-2 Workspaces и terminal UX.md>) — parent package.
- [PAR-FS-012](<../../Paradigms/Архитектура/Рабочие области и интерфейс/02 Terminal-first интерфейс.md>) — product UX.
- [Нормативные шаблоны интерфейса](<../UX/01 Нормативные шаблоны интерфейса.md>) — visual and routing contract.

## Входы и результат

Входы: startup context, catalog/workspace status, typed user event и application result. Результат: следующий terminal state и один или несколько `terminal-dialogue` documents без direct stdout/stderr formatting из FastSearch application code.

## State machine

```text
START
  -> WORKSPACE_PICKER | WORKSPACE_OPEN
WORKSPACE_PICKER
  -> WORKSPACE_CREATE | WORKSPACE_OPEN | EXIT
WORKSPACE_CREATE
  -> DISCOVERY_REVIEW -> WORKSPACE_OPEN | BACK
WORKSPACE_OPEN
  -> INDEX_TRANSITION -> SEARCH_READY
SEARCH_READY
  -> SEARCH_RESULTS | EMPTY | ERROR | SOURCES | INDEX | COMPARE_READINESS | WORKSPACE_PICKER | EXIT
SEARCH_RESULTS
  -> RESULT_DETAIL | RELATED | SEARCH_READY | BACK
COMPARE_READINESS
  -> COMPARE_UPDATE | COMPARE_QUERY | SEARCH_READY | BACK
COMPARE_UPDATE
  -> COMPARE_READINESS | ERROR | BACK
COMPARE_QUERY
  -> COMPARE_RESULTS | COMPARE_PARTIAL | EMPTY | ERROR | BACK
COMPARE_RESULTS | COMPARE_PARTIAL
  -> RESULT_DETAIL | EXPERIMENT_JUDGMENT | COMPARE_QUERY | SEARCH_READY | BACK
```

Bare text имеет смысл query только в `SEARCH_READY`. Глобальные commands используют единый явный prefix; numbered/local actions интерпретируются только активным document state.

## Rendering boundary

`terminal-dialogue` владеет:

- uppercase heading policy, palette, two-space content indent, adaptive blue separator и timestamp;
- prompts, validation feedback, cancellation и terminal echo;
- welcome, selection, progress, result, empty-state, notice и error documents;
- narrow-terminal degradation, `NO_COLOR` и snapshot-stable plain rendering.

FastSearch владеет domain labels, workspace/source data, result ranking/provenance, allowed actions и transition decisions. Ручная сборка ANSI, separator, timestamp, indentation или framework chrome в FastSearch запрещена boundary test.

## Ошибки и граничные случаи

- Нет известных областей.
- Известная область перемещена или несовместима по schema.
- Ноль source contours.
- Index empty/stale/partial/unavailable.
- Query не дал результатов.
- Result устарел между list и detail.
- Terminal слишком узкий, color отключён или input закрыт.
- Update/rebuild прерван; следующий запуск объясняет recovery state.

## Инварианты

- Service/model paths отсутствуют в basic onboarding.
- Optional provider failure не закрывает доступный lexical/structural contour.
- Full rebuild не выполняется скрыто внутри query handling.
- Comparison partition update не выполняется скрыто внутри query handling и требует отдельного подтверждения.
- Comparison session не меняет active model обычного `SEARCH_READY`.
- Любое состояние имеет явные next actions, back/cancel и exit path.
- Result actions локальны текущему result document.
- JSON/direct CLI contract не зависит от terminal renderer.

## Связь с кодом и проверки

State transitions реализованы функциями router boundary, а render models полностью состоят из typed `terminal-dialogue` documents. Workspace next step выводится по freshness: `CURRENT → query`, `STALE → /index update`, `DEGRADED → /status + /index rebuild`, `NotConfigured → /sources set`; bare query вне `CURRENT` блокируется до search progress как `SEARCH_NOT_READY`. Contract tests покрывают picker, create/discovery, 0/1/2 contours, index preview/cancel, stale search guard, results/detail/navigation, unknown-command recovery, EOF, redirected output и `NO_COLOR`; boundary scan запрещает direct output. Exhaustive transition-table остаётся quality gap.

## Состояние реализации

Current console использует `ChatSession::standard`, recent-workspace picker/current-directory resolution, create/discovery review, source rescan/edit, visible indexing, guarded bare-text query, typed result pager, `/open <номер>`, `/related <номер>` и controlled switch/exit. `/model` и comparison readiness используют framework-owned responsive `TableDocument`: FastSearch передаёт cells, но не tab/padding; narrow fallback выбирает библиотека. Вложенный `/compare` router добавляет read-only readiness, подтверждённый `/update`, единый query, model blocks, `/open A1|L1`, `/back` и partial failures, не изменяя active model. Service/index/model paths отсутствуют в basic onboarding; direct CLI и JSON не зависят от renderer.

Command guidance также принадлежит framework: каждое доступное действие
передаётся отдельным `ActionItem` внутри `NextStep` и выводится отдельной
строкой. Склеенные application strings вида `/a · /b · /c` не являются
допустимым render contract.

Path presentation является частью human renderer boundary. `/sources`
показывает нормализованные platform paths без внутренних Windows extended-path
prefixes, а также явно описывает отсутствие отдельного режима и команды
возврата, справки и завершения приложения.
