/**
 * The plug's declaration (product scope §8) stays the truth: `requires` names
 * exactly the unit exports this plug's code calls, the members of its typed
 * unit (unit.ts `WaterUnit`, which every call is checked against),
 * and each one is called somewhere in the plug.
 */
import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const plug = JSON.parse(readFileSync(join(here, 'plug.json'), 'utf8')) as {
	schema: string; component: string; namespace: string; requires: string[]; tab?: { namespace: string };
};

it('requires exactly the members of the unit the plug calls', () => {
	const types = readFileSync(join(here, 'unit.ts'), 'utf8');
	const body = types.split('export type WaterUnit = {')[1].split('\n};')[0];
	const members = [...body.matchAll(/^\t(\w+)\??\(/gm)].map((match) => match[1]).sort();
	expect(plug.schema).toBe('ds.plugin-plug/v1');
	expect([plug.component, plug.namespace]).toEqual(['water', 'water']);
	// No standing area while the Kernel's built-in Water area serves (until the W2-switch).
	expect(plug.tab).toBeUndefined();
	expect(plug.requires).toEqual(members);
});

it('calls every export it requires', () => {
	const code = readdirSync(here)
		.filter((name) => /\.(ts|svelte)$/.test(name) && !name.endsWith('.test.ts'))
		.map((name) => readFileSync(join(here, name), 'utf8'))
		.join('\n');
	for (const name of plug.requires) {
		expect(new RegExp(`\\.${name}\\??\\.?\\(`).test(code), name).toBe(true);
	}
});
