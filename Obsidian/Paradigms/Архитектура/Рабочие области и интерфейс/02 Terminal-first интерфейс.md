---
id: "PAR-FS-012"
title: "Terminal-first интерфейс FastSearch"
status: "принято"
implementation_stage: "текущее"
tdr_refs: ["TDR-FS-2.3"]
tdr_coverage: "прямое"
updated: "2026-08-16"
---
# Terminal-first интерфейс FastSearch

[← Рабочие области и интерфейс](<00 Рабочие области и интерфейс.md>)

## Статус

Terminal-first human-interface реализован как основной запуск FastSearch. Обязательный мастер технических путей удалён из human flow.

## Контекст

Обязательный ввод document root, code root, service root и optional model root раскрывает внутреннее устройство до получения пользы. Поиск должен быть главным действием, а настройка, индексация и обслуживание — контекстными ветвями с progressive disclosure.

## Парадигма

FastSearch открывается экраном известных рабочих областей либо сразу активирует область, если запуск выполнен внутри её canonical root. После открытия пользователь вводит обычный текст как поисковый запрос. Глобальные команды ограничены навигацией, состоянием и обслуживанием; действия над результатом остаются локальными результату.

Human rendering полностью делегируется `terminal-dialogue`. FastSearch передаёт типизированные документы, поля, результаты, progress и ошибки и не собирает framework chrome вручную.

## Основной маршрут

1. Выбрать недавнюю область или создать новую.
2. При создании указать один workspace root.
3. Автоматически обнаружить document/code roots и показать проверяемое резюме.
4. Разрешить пропустить любой контур и явно показать состояние с нулём, одним или двумя контурами.
5. Создать или открыть `.fastsearch`, проверить freshness и выполнить видимую initial/update indexing phase при согласованной policy.
6. Принять поисковый запрос.
7. Показать типизированные результаты, provenance, freshness и локальные действия.

## Границы

- `service`, SQLite path, index directory и E5 root не запрашиваются в основном onboarding.
- E5 и другие optional providers находятся в advanced capabilities и не блокируют lexical/structural search.
- `rebuild` остаётся явным maintenance action; search request не скрывает полный rebuild.
- Empty, stale, partial, unavailable и current отображаются различимо.
- Direct deterministic CLI и JSON остаются machine/operator surface и не получают human chrome.

## Инварианты

- Первое полезное действие после открытия готовой области — поисковый запрос.
- Пользователь вводит один workspace root, а не внутренние storage paths.
- Enter, cancel, back и exit имеют одинаковую семантику на всех экранах.
- Заголовки, отступы, separator, timestamp, palette, progress, result, empty-state и error принадлежат `terminal-dialogue`.
- Bare text в search context является запросом; глобальные команды имеют явный префикс.
- Result actions не раздувают глобальное command namespace.

## Связи

- [Рабочая область и два контура источников](<01 Рабочая область и два контура источников.md>) — состояния и source admission.
- [Нормативные шаблоны интерфейса](<../../../Docs/UX/01 Нормативные шаблоны интерфейса.md>) — обязательная форма экранов и переходов.
- [TDR-FS-2.3](<../../../Docs/TDR/TDR-FS-2.3 Terminal routing и terminal-dialogue.md>) — state machine и rendering boundary.

## Связь с реализацией

Current console использует `ChatSession::standard`, workspace picker, create/discovery review, 0/1/2-contour states, visible index transition, bare-text search, typed results, stable numbered navigation и contextual commands. `tests/cli_ux.rs` подтверждает onboarding, picker, optional contours, query/result/detail, recovery, preview/cancel и `NO_COLOR`; `tests/terminal_dialogue_boundary.rs` запрещает direct terminal rendering. Остаточный quality gap: отдельные golden fixtures для narrow terminal и специального partial-capability screen.
