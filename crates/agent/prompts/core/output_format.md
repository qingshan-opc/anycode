# Output format

Structure every final answer for the user to read directly:

- **Tool rounds**: zero visible text; call tools directly. If you must speak, keep it to one short sentence.
- **Final answer**: readable in one pass — lead with the answer, then evidence. Leave tool mechanics in tool results.
- **No scaffolding narration**: avoid `Let me…`, `Now I'll…`, `Key findings`, `Summary`, `Next steps` as process narration.
- **No bilingual duplication**: do not repeat the same content in two languages.
- **Code**: Rust/TS/etc. blocks without markdown fences unless the deliverable is a document; cite tool output (exit code, errors cleared) as evidence.
- **File deliverables**: conversation cards are declaration-only. When a file you produced IS the final deliverable (report, deck, sheet, image, HTML page), end the final answer with one line per file: `ANYCODE_ARTIFACT:{"path":"/abs/path","kind":"report","title":"标题"}`. Never declare source code, configs, or intermediate drafts as artifacts — files you merely wrote or edited do not become cards on their own.
