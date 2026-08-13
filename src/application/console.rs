use std::io::{self, BufRead, IsTerminal, Write};

use super::{
    cli::{execute_command, production_command, render_outcome, CommandAction},
    OutputFormat,
};

const GREEN: &str = "\x1b[32m";
const RESET: &str = "\x1b[0m";

#[derive(Clone, Debug)]
struct SessionContext {
    documents: String,
    code: String,
    service: String,
    e5: Option<String>,
}

/// Runs the human-oriented console using standard input and output.
pub fn run_interactive() -> io::Result<()> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    run_interactive_with(&mut input, &mut output, stdout.is_terminal())
}

fn run_interactive_with(
    input: &mut impl BufRead,
    output: &mut impl Write,
    terminal: bool,
) -> io::Result<()> {
    writeln!(
        output,
        "{}",
        accent("FastSearch — поиск по документам и коду", terminal)
    )?;
    writeln!(
        output,
        "Настройте рабочий контекст один раз, затем вводите короткие команды.\n"
    )?;
    let Some(mut context) = read_context(input, output)? else {
        writeln!(output, "Ввод завершён. FastSearch закрыт.")?;
        return Ok(());
    };
    writeln!(output, "\n{}", accent("Контекст готов", terminal))?;
    writeln!(output, "{}\n", command_summary())?;

    let mut format = OutputFormat::Human;
    loop {
        write!(output, "fastsearch> ")?;
        output.flush()?;
        let Some(line) = read_line(input)? else {
            writeln!(output, "\nВвод завершён. FastSearch закрыт.")?;
            return Ok(());
        };
        let command = line.trim();
        if command.is_empty() {
            writeln!(output, "Введите команду или `help`. Для выхода: `exit`.")?;
            continue;
        }
        if matches!(command, "exit" | "quit" | "выход") {
            writeln!(output, "FastSearch закрыт.")?;
            return Ok(());
        }
        if matches!(command, "help" | "--help" | "-h" | "помощь") {
            writeln!(output, "{}\n", interactive_help())?;
            continue;
        }
        if matches!(command, "version" | "--version" | "-V") {
            writeln!(output, "{}\n", version_text())?;
            continue;
        }
        if command == "context" {
            writeln!(output, "{}\n", render_context(&context))?;
            continue;
        }
        if command == "context set" {
            writeln!(
                output,
                "Перенастройка контекста. Enter сохраняет текущее значение."
            )?;
            if let Some(updated) = update_context(input, output, &context)? {
                context = updated;
                writeln!(output, "Контекст обновлён.\n")?;
            } else {
                writeln!(output, "Ввод завершён. FastSearch закрыт.")?;
                return Ok(());
            }
            continue;
        }
        if matches!(command, "json" | "--json") {
            format = if format == OutputFormat::Json {
                OutputFormat::Human
            } else {
                OutputFormat::Json
            };
            writeln!(
                output,
                "Режим вывода: {}.\n",
                if format == OutputFormat::Json {
                    "JSON"
                } else {
                    "обычный текст"
                }
            )?;
            continue;
        }

        match interactive_action(command) {
            Ok(action) => {
                if format == OutputFormat::Human {
                    writeln!(output, "Выполняю команду…")?;
                    output.flush()?;
                }
                let command = production_command(
                    &context.documents,
                    &context.code,
                    &context.service,
                    context.e5.as_ref(),
                    action,
                );
                match execute_command(command).map(|outcome| render_outcome(outcome, format)) {
                    Ok(result) if format == OutputFormat::Human => {
                        writeln!(output, "{}\n", accent_first_line(&result, terminal))?
                    }
                    Ok(result) => writeln!(output, "{result}\n")?,
                    Err(error) if format == OutputFormat::Json => {
                        writeln!(output, "{}\n", error.render_json())?
                    }
                    Err(error) => writeln!(
                        output,
                        "Ошибка: {}\nПодсказка: введите `help`.\n",
                        error.message()
                    )?,
                }
            }
            Err(message) => writeln!(output, "Ошибка: {message}\nПодсказка: введите `help`.\n")?,
        }
    }
}

fn read_context(
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> io::Result<Option<SessionContext>> {
    let Some(documents) = required_prompt(
        input,
        output,
        "Папка документов (Markdown/TSV). Пример: C:\\Project\\docs. Enter не допускается; `exit` — отмена.\ndocuments> ",
    )?
    else {
        return Ok(None);
    };
    let Some(code) = required_prompt(
        input,
        output,
        "Папка исходного кода. Пример: C:\\Project\\src. Enter не допускается; `exit` — отмена.\ncode> ",
    )?
    else {
        return Ok(None);
    };
    let Some(service) = required_prompt(
        input,
        output,
        "Папка служебного индекса. Пример: C:\\Project\\.fastsearch. Enter не допускается; `exit` — отмена.\nservice> ",
    )?
    else {
        return Ok(None);
    };
    write!(
        output,
        "Папка модели E5 (необязательно). Пример: C:\\Models\\e5-small. Enter — не подключать векторный канал; `exit` — отмена.\ne5> "
    )?;
    output.flush()?;
    let Some(e5) = read_line(input)? else {
        return Ok(None);
    };
    if is_exit(e5.trim()) {
        return Ok(None);
    }
    Ok(Some(SessionContext {
        documents,
        code,
        service,
        e5: nonblank(e5),
    }))
}

fn update_context(
    input: &mut impl BufRead,
    output: &mut impl Write,
    current: &SessionContext,
) -> io::Result<Option<SessionContext>> {
    let Some(documents) = optional_prompt(input, output, "documents", &current.documents)? else {
        return Ok(None);
    };
    let Some(code) = optional_prompt(input, output, "code", &current.code)? else {
        return Ok(None);
    };
    let Some(service) = optional_prompt(input, output, "service", &current.service)? else {
        return Ok(None);
    };
    let e5_current = current.e5.as_deref().unwrap_or("не задана");
    write!(
        output,
        "e5 [{e5_current}] (Enter — сохранить; `-` — очистить)> "
    )?;
    output.flush()?;
    let Some(e5_input) = read_line(input)? else {
        return Ok(None);
    };
    let e5 = match e5_input.trim() {
        "" => current.e5.clone(),
        "-" => None,
        value => Some(value.to_owned()),
    };
    Ok(Some(SessionContext {
        documents,
        code,
        service,
        e5,
    }))
}

fn required_prompt(
    input: &mut impl BufRead,
    output: &mut impl Write,
    prompt: &str,
) -> io::Result<Option<String>> {
    loop {
        write!(output, "{prompt}")?;
        output.flush()?;
        let Some(value) = read_line(input)? else {
            return Ok(None);
        };
        if is_exit(value.trim()) {
            return Ok(None);
        }
        if !value.trim().is_empty() {
            return Ok(Some(value.trim().to_owned()));
        }
        writeln!(output, "Значение обязательно. Введите полный путь.")?;
    }
}

fn optional_prompt(
    input: &mut impl BufRead,
    output: &mut impl Write,
    name: &str,
    current: &str,
) -> io::Result<Option<String>> {
    write!(output, "{name} [{current}] (Enter — сохранить)> ")?;
    output.flush()?;
    let Some(value) = read_line(input)? else {
        return Ok(None);
    };
    Ok(Some(if value.trim().is_empty() {
        current.to_owned()
    } else {
        value.trim().to_owned()
    }))
}

fn read_line(input: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut line = String::new();
    if input.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    Ok(Some(line.trim_end_matches(['\r', '\n']).to_owned()))
}

fn interactive_action(command: &str) -> Result<CommandAction, &'static str> {
    let mut words = command.split_whitespace();
    let Some(first) = words.next() else {
        return Err("пустая команда");
    };
    match first {
        "init" | "status" => {
            if words.next().is_some() {
                return Err("эта команда не принимает параметры");
            }
            Ok(if first == "init" {
                CommandAction::Init
            } else {
                CommandAction::Status
            })
        }
        "index" => {
            let Some(action @ ("update" | "rebuild")) = words.next() else {
                return Err("после `index` укажите `update` или `rebuild`");
            };
            if words.next().is_some() {
                return Err("после действия лишние параметры");
            }
            Ok(CommandAction::Index {
                rebuild: action == "rebuild",
            })
        }
        "search" => {
            let Some(mode @ ("balanced" | "current" | "design")) = words.next() else {
                return Err("укажите режим balanced, current или design, затем текст запроса");
            };
            let query = words.collect::<Vec<_>>().join(" ");
            if query.is_empty() {
                return Err("после режима введите текст запроса");
            }
            let mode = match mode {
                "balanced" => crate::domain::SearchMode::Balanced,
                "current" => crate::domain::SearchMode::Current,
                "design" => crate::domain::SearchMode::Design,
                _ => unreachable!("validated interactive mode"),
            };
            Ok(CommandAction::Search { mode, text: query })
        }
        "get" | "related" => {
            let id = words.collect::<Vec<_>>().join(" ");
            if id.is_empty() {
                return Err("укажите стабильный ID записи");
            }
            Ok(if first == "get" {
                CommandAction::Get { id }
            } else {
                CommandAction::Related { id }
            })
        }
        _ => Err("неизвестная команда"),
    }
}

#[must_use]
pub fn help_text() -> &'static str {
    "FastSearch — локальный поиск по документам и исходному коду.\n\nЗапуск:\n  fastsearch                       интерактивный мастер и чат\n  fastsearch chat                  то же самое явно\n  fastsearch [--json] <команда>    прямой детерминированный вызов\n  fastsearch --help                эта справка\n  fastsearch --version             версия программы\n\nПрямые команды:\n  init <documents> <code> <service> [e5-root]\n  index update|rebuild <documents> <code> <service> [e5-root]\n  search <documents> <code> <service> <balanced|current|design> <query> [e5-root]\n  get|related <documents> <code> <service> <stable-id> [e5-root]\n  status <documents> <code> <service> [e5-root]\n\n`--json` можно указывать в любой позиции до разделителя `--`. Без него прямые команды сохраняют технический формат для совместимости."
}

#[must_use]
pub fn version_text() -> String {
    format!("FastSearch {}", env!("CARGO_PKG_VERSION"))
}

fn interactive_help() -> &'static str {
    "Команды чата:\n  search balanced <текст>  поиск; режимы: balanced, current, design\n  get <stable-id>           показать запись\n  related <stable-id>       показать связи\n  index update              обновить индекс\n  index rebuild             перестроить индекс\n  status                    состояние и возможности\n  context                   показать пути\n  context set               изменить пути\n  json                      переключить обычный текст / JSON\n  help                      эта памятка\n  exit                      выход"
}

fn command_summary() -> &'static str {
    "Введите команду, например `search balanced архитектура`. Справка: `help`. Выход: `exit`."
}

fn render_context(context: &SessionContext) -> String {
    format!(
        "Рабочий контекст\n  Документы: {}\n  Код: {}\n  Служебные данные: {}\n  E5: {}",
        context.documents,
        context.code,
        context.service,
        context.e5.as_deref().unwrap_or("не настроена")
    )
}

fn nonblank(value: String) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.trim().to_owned())
}

fn is_exit(value: &str) -> bool {
    matches!(value, "exit" | "quit" | "выход")
}

fn accent(text: &str, terminal: bool) -> String {
    if terminal && std::env::var_os("NO_COLOR").is_none() {
        format!("{GREEN}{text}{RESET}")
    } else {
        text.to_owned()
    }
}

fn accent_first_line(text: &str, terminal: bool) -> String {
    let Some((heading, body)) = text.split_once('\n') else {
        return accent(text, terminal);
    };
    format!("{}\n{body}", accent(heading, terminal))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{interactive_action, run_interactive_with, CommandAction};

    #[test]
    fn paths_with_spaces_are_kept_as_complete_prompt_values() {
        let input = b"C:\\My docs\nC:\\My code\nC:\\My state\n\ncontext\nexit\n";
        let mut output = Vec::new();
        run_interactive_with(&mut Cursor::new(input), &mut output, false).expect("session");
        let output = String::from_utf8(output).expect("UTF-8");
        assert!(output.contains("Документы: C:\\My docs"));
        assert!(output.contains("Код: C:\\My code"));
        assert!(!output.contains("\x1b["));
    }

    #[test]
    fn human_search_translates_directly_to_a_typed_command_action() {
        let action = interactive_action("search current words with spaces").expect("typed action");

        assert!(matches!(action, CommandAction::Search { .. }));
    }
}
