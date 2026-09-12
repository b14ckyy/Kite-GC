# This script reads the English locale JSON file, builds a dictionary of nested paths for all keys, and writes
# the result to an output JSON file -- locales/xx.json.
#
# Usage: python tools/i18n-key-paths.py
#
# Then add the xx 'locale' to lib/i18n/index.ts
#
#   register('xx', () => import('./locales/xx.json'));
#
# and add
#
#   { code: 'xx', label: 'i18n-paths' },
#
# to SUPPORTED_LOCALES (also in lib/i18n/index.ts)
#
# After you switch to the xx locale in the app, you can see all the i18n keys in the UI.
# Enjoy translating!
#
import json

LOCALES_PATH = "src/lib/i18n/locales"
FNAME_EN = f"{LOCALES_PATH}/en.json"  # en hardcoded as the master reference for all string keys
OUTPUT_FNAME = f"{LOCALES_PATH}/xx.json"


def build_paths(obj, path=()):
    """Recursively build paths for nested dictionaries."""
    if isinstance(obj, dict):
        return {
            key: build_paths(value, path + (key,))
            for key, value in obj.items()
        }
    return ".".join(path)


result = build_paths(json.load(open(FNAME_EN, 'r', encoding="utf-8")))
json.dump(result, open(OUTPUT_FNAME, 'w', encoding="utf-8"), indent=2)
print(f"Key paths written to {OUTPUT_FNAME}")
