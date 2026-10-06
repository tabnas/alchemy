# @tabnas/alchemy

The TypeScript implementation of Alchemy, the small transformation language
for streaming structured data through the Tabnas parser and transducer stack.

```sh
npm install @tabnas/alchemy @tabnas/transduce @tabnas/render @tabnas/parser @tabnas/json
```

`@tabnas/parser` (the engine) and `@tabnas/json` are optional peers: the
language (`@tabnas/alchemy`) needs both, and the shared types
(`@tabnas/alchemy/shared`, which transduce and render build on) need
neither, so a package that takes only the shared types does not pull them
in. Install them wherever a program is compiled.

Alchemy compiles a program onto stages it declares and does not implement:
`Routers` and `Renderers`. `@tabnas/transduce` exports `routers` and
`@tabnas/render` exports `renderers`, and a host passes both to `compile`:

```ts
import { compile } from '@tabnas/alchemy'
import { routers } from '@tabnas/transduce'
import { renderers } from '@tabnas/render'

const program = compile(source, 'export.alc', { routers, renderers })
```

The types alchemy, transduce and render share (the event and table
protocols, `Fail` and its codes, `Limits`, selectors, the text boundary and
the renderers' options) are this package's, under `@tabnas/alchemy/shared`,
an entry that loads them and nothing else; the main entry re-exports them.

The `alchemy` command is `@tabnas/alchemy-cli`. See the
[project README](https://github.com/tabnas/alchemy#readme) for the language
guide and API examples.

This package includes its TypeScript sources under `src/` alongside the
compiled JavaScript and declarations under `dist/`.
