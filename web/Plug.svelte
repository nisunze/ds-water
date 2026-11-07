<script lang="ts">
	/**
	 * Water's one plug (product scope §7): mounted once by the root layout,
	 * it hands the Kernel the Water unit's commands, with this unit as their
	 * `plugin` door, and then its Design area. The Kernel admits both,
	 * performs every project read and write, and places the area; while it is
	 * not admitted (an older unit or Kernel), the built-in Water arm answers.
	 * Unmounting releases them.
	 */
	import { onMount } from 'svelte';
	import { registerPluginCommands, registerPluginRibbon } from '$lib/command-line/plugins';
	import { loadWaterUnit, waterHandler } from './unit';

	onMount(() => {
		let release: (() => void) | null = null;
		let gone = false;
		// After the first paint: the unit loads when the page is idle.
		const start = () =>
			void loadWaterUnit().then(async (unit) => {
				if (!unit || gone) return;
				const done = await registerPluginCommands(unit.pluginManifest(), waterHandler(unit));
				if (gone) return done();
				release = done;
				if (unit.pluginRibbon) await registerPluginRibbon(unit.pluginRibbon());
			});
		const idle = typeof requestIdleCallback === 'function';
		const waiting = idle ? requestIdleCallback(start) : setTimeout(start, 1);
		return () => {
			gone = true;
			if (idle) cancelIdleCallback(waiting as number);
			else clearTimeout(waiting);
			release?.();
		};
	});
</script>
