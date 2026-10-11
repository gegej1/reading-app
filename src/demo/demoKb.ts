// Demo knowledge base for the in-memory store (web build without a real KB,
// plain `pnpm dev`). It reuses the synthetic test fixtures: invented content
// in the real KB formats. Never put real knowledge-base content here.

import { fixtureKb } from "../domain/__fixtures__";

export const demoKb: Readonly<Record<string, string>> = fixtureKb;
