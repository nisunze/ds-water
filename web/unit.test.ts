import { beforeEach, expect, it, vi } from 'vitest';

const page = vi.hoisted(() => ({ unit: null as Record<string, unknown> | null, asked: [] as string[] }));

vi.mock('$lib/ds/client', () => ({
	pageUnit: async (name: string) => {
		page.asked.push(name);
		if (!page.unit) throw new Error('water_engine_missing');
		return page.unit;
	},
}));

beforeEach(() => {
	vi.resetModules();
	page.unit = null;
	page.asked = [];
});

async function load() {
	const { loadWaterUnit } = await import('./unit');
	return loadWaterUnit();
}

it('plugs the page unit that carries its command door', async () => {
	const calls: [string, string][] = [];
	page.unit = {
		pluginManifest: () => '{}',
		pluginCall: (command: string, request: string) => {
			calls.push([command, request]);
			return JSON.stringify({ status: 'ok', data: { served: 'unit' } });
		},
	};
	const unit = await load();
	expect(page.asked).toEqual(['water']);
	const { waterHandler } = await import('./unit');
	const answer = await waterHandler(unit!)('water.run', { operation: 'identity' });
	expect(answer).toEqual({ status: 'ok', data: { served: 'unit' } });
	expect(calls).toEqual([['water.run', '{"operation":"identity"}']]);
});

it('gives no plug for an older unit, so the built-in Water commands answer', async () => {
	page.unit = { water_invoke_json: () => '{}' };
	expect(await load()).toBeNull();
});

it('gives no plug when no Water unit loads', async () => {
	expect(await load()).toBeNull();
	expect(page.asked).toEqual(['water']);
});
