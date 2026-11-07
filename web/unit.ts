/**
 * The page's governed Water unit, the very instance the page's unit door
 * calls (the published unit by digest on the Web, Core's on the Desktop). A
 * unit without its command door (an older release) gives no plug, so the
 * Kernel's own Water arm keeps answering.
 */
export type WaterUnit = {
	pluginManifest(): string;
	pluginRibbon?(): string;
	pluginCall(command: string, request: string): string;
};

/** The loaded unit when it carries its command door, else null. */
export async function loadWaterUnit(): Promise<WaterUnit | null> {
	try {
		const { pageUnit } = await import('$lib/ds/client');
		const unit = (await pageUnit('water')) as Partial<WaterUnit>;
		if (typeof unit.pluginManifest !== 'function' || typeof unit.pluginCall !== 'function') {
			console.info('[water] the loaded unit has no command door; the built-in Water commands answer');
			return null;
		}
		return unit as WaterUnit;
	} catch (error) {
		console.info('[water] no Water unit was loaded:', error);
		return null;
	}
}

/** The `plugin` handler: one door call into the unit, its answer as the Kernel reads it. */
export function waterHandler(unit: WaterUnit) {
	return async (command: string, request: Record<string, unknown>) =>
		JSON.parse(unit.pluginCall(command, JSON.stringify(request)));
}
