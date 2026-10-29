#!/usr/bin/env python3
"""Adds to the APK the configuration changes the SDK does not let you write by name.

Android recreates the activity on every configuration change the manifest does not declare. For a
Rust NativeActivity, recreating it means hanging: `onDestroy` waits for `android_main`
to return, but Iced's loop never ends and "App not responding" arrives. The public names
(`android:configChanges` in android/Cargo.toml) cover almost everything. Two hidden bits are left out:

- `assetsPaths` (0x80000000): theme overlays change, for example background colors on
  Android 12 and later;
- `windowConfiguration` (0x20000000): multi-window and freeform windows.

`aapt` rejects both these names and a numeric value. The script adds them to the value already compiled
into the binary manifest and rewrites the APK without a signature. Then `scripts/android-apk.sh` aligns
and signs it again.

Con un terzo argomento (`classes.dex`) aggiunge anche il codice Java di Psytify
(`android/java/`, compilato da `scripts/android-apk.sh`): cargo-apk compila solo Rust.

Usage: android-manifest.py INPUT.apk OUTPUT.apk [classes.dex]
"""

import struct
import sys
import zipfile

CONFIG_CHANGES = 0x0101001F  # android:configChanges
HIDDEN_FLAGS = 0x80000000 | 0x20000000
TYPE_INT_HEX = 0x11

RES_STRING_POOL = 0x0001
RES_XML_RESOURCE_MAP = 0x0180
RES_XML_START_ELEMENT = 0x0102


def patch_manifest(data: bytearray) -> int:
    """Somma i bit nascosti a ogni `configChanges`; restituisce quanti attributi ha cambiato."""
    _, header_size, total = struct.unpack_from("<HHI", data, 0)
    resource_ids: list[int] = []
    patched = 0
    offset = header_size
    while offset < total:
        kind, chunk_header, size = struct.unpack_from("<HHI", data, offset)
        if kind == RES_XML_RESOURCE_MAP:
            count = (size - chunk_header) // 4
            resource_ids = list(struct.unpack_from(f"<{count}I", data, offset + chunk_header))
        elif kind == RES_XML_START_ELEMENT:
            # ResXMLTree_attrExt: ns, name, attributeStart, attributeSize, attributeCount, ...
            ext = offset + chunk_header
            attr_start, attr_size, attr_count = struct.unpack_from("<HHH", data, ext + 8)
            for index in range(attr_count):
                attr = ext + attr_start + index * attr_size
                name = struct.unpack_from("<I", data, attr + 4)[0]
                if name >= len(resource_ids) or resource_ids[name] != CONFIG_CHANGES:
                    continue
                data_type = data[attr + 15]
                value = struct.unpack_from("<I", data, attr + 16)[0]
                if data_type != TYPE_INT_HEX:
                    raise SystemExit(f"configChanges ha un tipo inatteso: {data_type:#x}")
                struct.pack_into("<I", data, attr + 16, value | HIDDEN_FLAGS)
                patched += 1
        offset += size
    return patched


def main() -> None:
    if len(sys.argv) not in (3, 4):
        raise SystemExit(__doc__)
    source, target = sys.argv[1], sys.argv[2]
    with zipfile.ZipFile(source) as apk, zipfile.ZipFile(target, "w") as out:
        if len(sys.argv) == 4:
            if "classes.dex" in apk.namelist():
                raise SystemExit("l'APK ha già un classes.dex: il codice Java si aggiunge una volta sola")
            out.write(sys.argv[3], "classes.dex", compress_type=zipfile.ZIP_DEFLATED)
        for info in apk.infolist():
            # The old v1 signature is no longer valid: it is signed again afterwards.
            if info.filename.startswith("META-INF/"):
                continue
            payload = apk.read(info)
            if info.filename == "AndroidManifest.xml":
                manifest = bytearray(payload)
                if patch_manifest(manifest) == 0:
                    raise SystemExit("configChanges not found in the manifest")
                payload = bytes(manifest)
            out.writestr(info, payload, compress_type=info.compress_type)


if __name__ == "__main__":
    main()
