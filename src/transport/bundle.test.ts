import { expect, test } from "bun:test";
import { build, type Rolldown } from "vite";

// 9.24: the fake service (`mock.ts`, and `replay.ts` it uses) is a chunk of its own, loaded only
// with `?mock` or outside Tauri: the production build's startup bundle does not carry it.
test("the production bundle loads the mock transport only on demand", async () => {
  const out = (await build({
    logLevel: "silent",
    build: { write: false, reportCompressedSize: false },
  })) as Rolldown.RolldownOutput;
  const chunks = out.output.filter((o): o is Rolldown.OutputChunk => o.type === "chunk");
  const holds = (c: Rolldown.OutputChunk, file: string) =>
    c.moduleIds.some((id) => id.endsWith(`/src/transport/${file}`));
  const entry = chunks.filter((c) => c.isEntry);
  expect(entry.length).toBe(1);
  // What the entry loads at startup: itself and the chunks it imports statically, transitively.
  const byName = new Map(chunks.map((c) => [c.fileName, c]));
  const startup = new Set<Rolldown.OutputChunk>();
  const visit = (c: Rolldown.OutputChunk | undefined) => {
    if (!c || startup.has(c)) return;
    startup.add(c);
    for (const name of c.imports) visit(byName.get(name));
  };
  visit(entry[0]);
  expect([...startup].some((c) => holds(c, "index.ts"))).toBe(true);
  const carriers = [...startup].filter((c) => holds(c, "mock.ts") || holds(c, "replay.ts"));
  expect(carriers.map((c) => c.fileName)).toEqual([]);
  const mock = chunks.filter((c) => holds(c, "mock.ts"));
  expect(mock.map((c) => c.isDynamicEntry)).toEqual([true]);
}, 120_000);
