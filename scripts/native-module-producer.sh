#!/usr/bin/env bash
# Water's own native module producer (ds.staged-module-producer/v1). It builds
# the standalone `ds-water` addon from the clean ds-water checkout containing
# this script and stages the target-suffixed executable. Release packaging
# admits and packages it; no host, publication or Core rule lives here.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WATER_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TARGET_TRIPLE="${1:-${DS_WATER_TARGET_TRIPLE:-}}"

case "${1:-}" in
	-h|--help)
		cat <<'USAGE'
usage: native-module-producer.sh [TARGET_TRIPLE]

Builds the `ds-water` standalone addon from the clean ds-water checkout that
contains this script and stages water-TARGET_TRIPLE[.exe] (component-named, as packaging expects).

  DS_MODULE_STAGE_DIR      required directory for the staged executable
  DS_RELEASE_PIN_DS_WATER  required SHA of this checkout, when pinning
  DS_WATER_TARGET_TRIPLE   override the host triple
  DS_WATER_SIDECAR_TARGET_DIR
                           persistent Cargo target for this addon (a warm
                           target only saves time; build-info still proves
                           the staged binary)
  CARGO_TARGET_DIR         honored when locating Cargo's compiled binary
USAGE
		exit 0 ;;
esac
(($# <= 1)) || { echo 'usage: native-module-producer.sh [TARGET_TRIPLE]' >&2; exit 2; }

for tool in cargo git node rustc; do
	command -v "$tool" >/dev/null 2>&1 || {
		echo "ERROR: $tool is required to build the Water addon." >&2
		exit 1
	}
done
[[ -f "$WATER_ROOT/Cargo.toml" && -f "$WATER_ROOT/Cargo.lock" \
	&& -f "$WATER_ROOT/apps/ds-water-cli/Cargo.toml" ]] || {
	echo "ERROR: ds-water is not a complete locked Rust checkout: $WATER_ROOT" >&2
	exit 1
}
STAGE_DIR="${DS_MODULE_STAGE_DIR:-}"
[[ -n "$STAGE_DIR" ]] || {
	echo 'ERROR: DS_MODULE_STAGE_DIR must name the staging directory for the Water addon.' >&2
	exit 1
}

ACTUAL_SOURCE_SHA="$(git -C "$WATER_ROOT" rev-parse --verify HEAD 2>/dev/null || true)"
[[ "$ACTUAL_SOURCE_SHA" =~ ^[0-9a-f]{40}$ ]] || {
	echo "ERROR: ds-water HEAD is not one exact lowercase Git SHA." >&2
	exit 1
}
PINNED_SOURCE_SHA="${DS_RELEASE_PIN_DS_WATER:-$ACTUAL_SOURCE_SHA}"
[[ "$PINNED_SOURCE_SHA" =~ ^[0-9a-f]{40}$ && "$ACTUAL_SOURCE_SHA" == "$PINNED_SOURCE_SHA" ]] || {
	echo "ERROR: ds-water HEAD does not match DS_RELEASE_PIN_DS_WATER." >&2
	exit 1
}
if [[ -n "$(git -C "$WATER_ROOT" status --porcelain --untracked-files=normal)" ]]; then
	echo "ERROR: ds-water has uncommitted inputs; refusing to label a dirty addon with $PINNED_SOURCE_SHA." >&2
	exit 1
fi

if [[ -z "$TARGET_TRIPLE" ]]; then
	TARGET_TRIPLE="$(rustc -vV | awk '/^host:/ { host=$2 } END { print host }')"
fi
[[ "$TARGET_TRIPLE" =~ ^[A-Za-z0-9_][A-Za-z0-9_.-]*$ ]] || {
	echo "ERROR: invalid Rust target triple for ds-water: $TARGET_TRIPLE" >&2
	exit 1
}

if [[ -n "${DS_WATER_SIDECAR_TARGET_DIR:-}" ]]; then
	export CARGO_TARGET_DIR="$DS_WATER_SIDECAR_TARGET_DIR"
fi

echo "  • building ds-water $PINNED_SOURCE_SHA for $TARGET_TRIPLE" >&2
(
	cd "$WATER_ROOT"
	DS_WATER_SOURCE_SHA="$PINNED_SOURCE_SHA" \
	CARGO_INCREMENTAL=0 \
		cargo build --locked --release --target "$TARGET_TRIPLE" \
		--package ds-water-cli --bin ds-water >&2
)

EXE_SUFFIX=""
[[ "$TARGET_TRIPLE" == *windows* ]] && EXE_SUFFIX=".exe"
# Cargo metadata is the authority for the directory Cargo just used.
WATER_TARGET_DIR="$(
	cd "$WATER_ROOT"
	cargo metadata --locked --format-version 1 --no-deps \
		| node -e '
			let raw = "";
			process.stdin.setEncoding("utf8");
			process.stdin.on("data", (chunk) => { raw += chunk; });
			process.stdin.on("end", () => {
				const value = JSON.parse(raw).target_directory;
				if (typeof value !== "string" || value.length === 0) process.exit(1);
				process.stdout.write(value);
			});
		'
)" || {
	echo "ERROR: Cargo did not report the target directory used for ds-water." >&2
	exit 1
}
case "$(uname -s 2>/dev/null || true)" in
	MINGW*|MSYS*|CYGWIN*) WATER_TARGET_DIR="$(cygpath -au -- "$WATER_TARGET_DIR")" ;;
esac
[[ "$WATER_TARGET_DIR" == /* ]] || {
	echo "ERROR: Cargo reported a non-absolute target directory for ds-water: $WATER_TARGET_DIR" >&2
	exit 1
}
BUILT_BIN="$WATER_TARGET_DIR/$TARGET_TRIPLE/release/ds-water$EXE_SUFFIX"
[[ -f "$BUILT_BIN" && ! -L "$BUILT_BIN" ]] || {
	echo "ERROR: compiled ds-water binary is not a regular file at Cargo's resolved target path: $BUILT_BIN" >&2
	exit 1
}

STAGED_BIN="$STAGE_DIR/water-$TARGET_TRIPLE$EXE_SUFFIX"
mkdir -p "$STAGE_DIR"
STAGED_TEMP="$STAGED_BIN.tmp.$$"
cleanup() { rm -f -- "$STAGED_TEMP"; }
trap cleanup EXIT INT TERM
cp -f -- "$BUILT_BIN" "$STAGED_TEMP"
mv -f -- "$STAGED_TEMP" "$STAGED_BIN"
trap - EXIT INT TERM

"$STAGED_BIN" build-info | node -e '
	let raw = "";
	process.stdin.setEncoding("utf8");
	process.stdin.on("data", (chunk) => { raw += chunk; });
	process.stdin.on("end", () => {
		const info = JSON.parse(raw);
		const [sha, triple] = process.argv.slice(1);
		const mismatches = [];
		const expect = (field, value) => {
			if (info[field] !== value) mismatches.push(`${field}: expected ${JSON.stringify(value)}, observed ${JSON.stringify(info[field])}`);
		};
		const requireDigest = (field) => {
			if (typeof info[field] !== "string" || !/^[0-9a-f]{64}$/.test(info[field])) {
				mismatches.push(`${field}: expected one lowercase SHA-256 digest, observed ${JSON.stringify(info[field])}`);
			}
		};
		expect("schema", "ds.engine-build/v1");
		expect("name", "ds-water-engine");
		expect("source_sha", sha);
		expect("target", triple);
		expect("profile", "release");
		requireDigest("cargo_lock_sha256");
		requireDigest("build_manifest_sha256");
		if (typeof info.rustc !== "string" || info.rustc.length === 0) mismatches.push("rustc: expected a compiler identity");
		if (!Array.isArray(info.schemas) || info.schemas.length === 0) mismatches.push("schemas: expected a non-empty array");
		if (typeof info.release !== "string" || !info.release.endsWith(`+${sha}`)) mismatches.push(`release: expected ds-water@<version>+${sha}`);
		if (mismatches.length > 0) {
			throw new Error(`staged ds-water build identity mismatch:\n- ${mismatches.join("\n- ")}`);
		}
	});
' "$PINNED_SOURCE_SHA" "$TARGET_TRIPLE"

echo "  ✓ staged compiled ds-water addon ($TARGET_TRIPLE)" >&2
printf '%s\n' "$STAGED_BIN"
