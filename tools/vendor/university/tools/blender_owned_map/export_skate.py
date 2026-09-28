"""Compatibility entry point for command-line SKATE exports.

The installable Blender addon owns the exporter implementation. Keeping this
small wrapper preserves existing background-build commands while ensuring the
GUI and command line always use the same exporter.
"""

from pathlib import Path
import sys


TOOL_ROOT = Path(__file__).resolve().parent
if str(TOOL_ROOT) not in sys.path:
    sys.path.insert(0, str(TOOL_ROOT))

from owned_world_material_addon.exporter import main


if __name__ == "__main__":
    main()
