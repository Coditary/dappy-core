pub fn help_text() -> &'static str {
    r#"Commands (gdb-style). Type `help <cmd>` for one command.

Execution:
  run, continue, cont, c       Resume execution
  next, n, step-over           Step over
  step, s, step-in             Step in
  finish, step-out             Step out

Breakpoints:
  break, b <file>:<line>       Set breakpoint [if EXPR] [hit N] [log MSG]
  trace, tbreak <file>:<line>  Logpoint [log MSG]
  clear <file>                 Clear all breakpoints in file
  clear <file>:<line>          Remove one breakpoint
  info break, info breakpoints List breakpoints (numbered)

Stack and source:
  bt, backtrace, stack, where  Stack trace (`bt full` for all frames)
  show, src, code [N]          Source around current line (N lines context)
  frame, f <id>                Select stack frame
  thread, t <id>               Select thread

Data:
  print, p, eval, expr <e>     Evaluate expression
  pp <expr>                    Pretty-print (with type when available)
  watch <expr>                 Add client-side watch (evaluated on stop)
  unwatch <expr>               Remove watch
  watch                        List watch expressions
  locals, info locals          Locals table for current frame
  scopes, info scopes          List scopes for current frame
  set <name> = <val>           Assign variable in current frame

Threads and filters:
  threads, info threads        List threads
  catch <filter> [if EXPR]     Enable exception breakpoint filter
  catch clear [filter]         Clear exception breakpoints
  info catch                   List exception filters and catches
  skip [list]                  List skip-path patterns
  skip add <pattern>           Auto-continue in matching sources
  skip clear [pattern]         Remove skip patterns
  smart-step [on|off]          Auto-continue through runtime/stdlib frames
  goto, jump <file>:<line>     Jump to source line (native or emulated)
  break function <name>        Function breakpoint (emulated via line BP)
  data-watch [add] <expr>      Emulated data breakpoint on expression
  data-watch remove <expr>     Remove data watch
  data-watch list              List data watches
  restart [frame <id>]         Restart from stack frame
  disasm <ref> [off] [count]   Disassemble memory at reference

Session:
  status                       Show remembered execution status
  sync                         Refresh state from session (late-join)
  capabilities, caps           Show adapter capabilities (JSON)
  colors, color [on|off]       Toggle syntax/ANSI colors (default off)
  version, ver                 Show dap-cli version
  help, h, ? [cmd]             Show this help or `help <cmd>`
  quit, q, exit                End session (attach: detach only)

Tips: Up/Down = command history. Prompt shows file:line when stopped.
NDJSON: dap-cli debug repl --ndjson [program]
Attach: dap-cli debug repl
"#
}

pub fn help_topic(topic: &str) -> Option<String> {
    let topic = topic.to_ascii_lowercase();
    let text = match topic.as_str() {
        "break" | "b" => {
            "break, b <file>:<line> [if EXPR] [hit N] [log MSG]\n  \
             Set a source breakpoint. Modifiers are emulated client-side when the adapter lacks support."
        }
        "trace" | "tbreak" => {
            "trace, tbreak <file>:<line> [log MSG]\n  Logpoint: prints MSG when hit (may auto-continue)."
        }
        "clear" => {
            "clear <file> or clear <file>:<line>\n  Remove breakpoints in a file or at one line."
        }
        "info" | "info break" | "info breakpoints" => {
            "info break, info breakpoints\n  List tracked breakpoints with numbers."
        }
        "bt" | "stack" | "backtrace" | "where" => {
            "bt, stack, where [full]\n  Stack trace. Runtime frames (site-packages, runpy, …) are hidden unless `full`."
        }
        "show" | "src" | "code" => "show, src, code [N]\n  Source listing with N lines of context (default 2).",
        "print" | "p" | "eval" | "expr" => {
            "print, p, eval, expr <expression>\n  Evaluate an expression in the current frame."
        }
        "pp" => "pp <expression>\n  Like print, with type annotation when colors are on.",
        "locals" | "info locals" => "locals, info locals\n  Show locals for the current frame in a table.",
        "scopes" | "info scopes" => "scopes, info scopes\n  List scopes for the current frame.",
        "set" => {
            "set <name> = <value>\n  Assign a variable in the current frame (not `s`, which steps)."
        }
        "colors" | "color" | "syntax-color" | "syntax-colors" => {
            "colors, color [on|off]\n  Toggle ANSI colors and light source highlighting (default off)."
        }
        "next" | "n" | "step-over" | "stepover" => "next, n\n  Step over the current line.",
        "step" | "s" | "step-in" | "stepin" => "step, s\n  Step into the current line.",
        "finish" | "step-out" | "stepout" => "finish, step-out\n  Step out of the current function.",
        "continue" | "cont" | "c" | "run" => "continue, run, c\n  Resume until the next stop.",
        "threads" | "info threads" => "threads, info threads\n  List threads.",
        "frame" | "f" => "frame, f <id>\n  Select a stack frame.",
        "thread" | "t" => "thread, t <id>\n  Select a thread.",
        "catch" | "info catch" => {
            "catch <filter> [if EXPR] | catch clear [filter]\n  Exception breakpoints. `info catch` lists filters."
        }
        "skip" => {
            "skip [list] | skip add <pattern> | skip clear [pattern]\n  Auto-continue in matching sources."
        }
        "goto" | "jump" => "goto, jump <file>:<line>\n  Jump execution to a source line.",
        "break function" | "function" => {
            "break function <name> | break function clear <name>\n  Function breakpoints (emulated when unsupported)."
        }
        "data-watch" | "dwatch" => {
            "data-watch [add] <expr> | data-watch remove <expr> | data-watch list\n  Emulated data breakpoints."
        }
        "restart" => "restart [frame <id>]\n  Restart execution from a stack frame.",
        "disasm" | "disassemble" => {
            "disasm <memory-reference> [offset] [count]\n  Disassemble instructions at a memory reference."
        }
        "watch" => "watch <expr> | unwatch <expr> | watch\n  Client-side watches evaluated on each stop.",
        "smart-step" | "smartstep" => {
            "smart-step [on|off]\n  Auto-continue through runtime/stdlib frames."
        }
        "status" => "status\n  Show remembered execution status.",
        "sync" => "sync\n  Refresh state from session (useful when attaching late).",
        "capabilities" | "caps" => "capabilities, caps\n  Show adapter capabilities JSON.",
        "version" | "ver" => "version, ver\n  Show dap-cli version.",
        "quit" | "q" | "exit" => "quit, q, exit\n  End session (attach mode: detach only, editor keeps running).",
        "help" | "h" | "?" => "help, h, ? [cmd]\n  Show command list or `help <cmd>` for one command.",
        _ => return None,
    };
    Some(text.to_string())
}
