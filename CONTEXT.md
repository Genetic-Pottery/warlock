## Vocabulary
- **Pact:** Any directory can be pacted. This means that it is parsed by the AI, a summary of the directory is written, and the contents are hashed so the TUI can visualize if the document is likely still true.
- **Fresh:** After pacting a directory it turns green in the TUI representing the AI has understood the directory and the WARLOCK.md produced from the pact is still true.
- **Stale:** When files are changed in a pacted directory it turns yellow in the TUI from the hash breaking, this means the AI likely does not understand the directory given the WARLOCK.md can possibly not refelct reality anymore.
- **Scope:** Any pacted directory can be given a scope, a scope says only a person who has a matching sigil can make changes to this directory.
- **Sigil:** Local strings in the warlock config which a user can freely change to ensure access to scoped directories. Scope and Sigil system is not a hard gate, it is a guardrail to prevent the AI from running away in a team setting, this can easily be side stepped because in real life there are many situations where you need to go around this. The final human gate is always a human reviewed PR so this method stays sane.
- **/brief:** Starts a conversation with warlock where you ideate a large portion of work. Warlock pushes back on your assumptions and once an understanding is reached you can write a document.
- **/write:** Writes a brief document to a chosen directory.
