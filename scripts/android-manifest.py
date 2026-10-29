#!/usr/bin/env python3
"""Aggiunge all'APK i cambi di configurazione che l'SDK non permette di scrivere per nome.

Android ricrea l'activity a ogni cambio di configurazione che il manifest non dichiara. Per una
NativeActivity in Rust ricrearla vuol dire bloccarsi: `onDestroy` aspetta che `android_main`
finisca, ma il ciclo di Iced non finisce mai e arriva «L'app non risponde». I nomi pubblici
(`android:configChanges` in android/Cargo.toml) coprono quasi tutto. Restano fuori due bit nascosti:

- `assetsPaths` (0x80000000): cambiano gli overlay del tema, per esempio i colori dello sfondo su
  Android 12 e successivi;
- `windowConfiguration` (0x20000000): finestre multiple e a forma libera.

`aapt` rifiuta sia questi nomi sia un valore numerico. Lo script li somma al valore già compilato
nel manifest binario e riscrive l'APK senza firma. Poi `scripts/android-apk.sh` lo allinea e lo
firma di nuovo.

Uso: android-manifest.py ENTRATA.apk USCITA.apk
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
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    source, target = sys.argv[1], sys.argv[2]
    with zipfile.ZipFile(source) as apk, zipfile.ZipFile(target, "w") as out:
        for info in apk.infolist():
            # La vecchia firma v1 non vale più: si rifirma dopo.
            if info.filename.startswith("META-INF/"):
                continue
            payload = apk.read(info)
            if info.filename == "AndroidManifest.xml":
                manifest = bytearray(payload)
                if patch_manifest(manifest) == 0:
                    raise SystemExit("configChanges non trovato nel manifest")
                payload = bytes(manifest)
            out.writestr(info, payload, compress_type=info.compress_type)


if __name__ == "__main__":
    main()
