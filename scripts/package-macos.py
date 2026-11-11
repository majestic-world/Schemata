"""Create an Apple Silicon flat installer from an OSXCross release build."""

import gzip
import json
import math
import plistlib
import shutil
import subprocess
import tempfile
import xml.etree.ElementTree as ET
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


def write_xml(path, element):
    ET.indent(element)
    ET.ElementTree(element).write(path, encoding="utf-8", xml_declaration=True)


def main():
    config = json.loads((ROOT / "src-tauri/tauri.conf.json").read_text())
    product = config["productName"]
    version = config["version"]
    identifier = config["identifier"]
    binary = "schematadb"
    minimum = "11.0"
    output = Path("/out")
    output.mkdir(exist_ok=True)

    # This pipeline embeds the frontend in the binary, like the native Tauri build.
    # Refuse unsupported additions rather than silently omit runtime resources.
    bundle = config.get("bundle", {})
    if any(bundle.get(key) for key in ("resources", "externalBin")) or bundle.get("macOS"):
        raise RuntimeError("Update the cross-packager for the new Tauri bundle resources/macOS settings.")

    with tempfile.TemporaryDirectory(prefix="schematadb-pkg-") as temporary:
        work = Path(temporary)
        payload_root = work / "root"
        app = payload_root / "Applications" / f"{product}.app"
        contents = app / "Contents"
        (contents / "MacOS").mkdir(parents=True)
        (contents / "Resources").mkdir()
        executable = contents / "MacOS" / binary
        shutil.copy2(ROOT / f"target/aarch64-apple-darwin/release/{binary}", executable)
        executable.chmod(0o755)
        shutil.copy2(ROOT / "src-tauri/icons/icon.icns", contents / "Resources/icon.icns")
        metadata = {
            "CFBundleDevelopmentRegion": "English",
            "CFBundleDisplayName": product,
            "CFBundleExecutable": binary,
            "CFBundleIdentifier": identifier,
            "CFBundleInfoDictionaryVersion": "6.0",
            "CFBundleName": product,
            "CFBundlePackageType": "APPL",
            "CFBundleShortVersionString": version,
            "CFBundleVersion": version,
            "CFBundleIconFile": "icon.icns",
            "LSMinimumSystemVersion": minimum,
            "NSHighResolutionCapable": True,
        }
        with (contents / "Info.plist").open("wb") as handle:
            plistlib.dump(metadata, handle)
        # Apple Silicon requires an executable signature, even for local builds.
        # This is ad-hoc signing, not Developer ID signing or notarization.
        subprocess.run(["rcodesign", "sign", str(app)], check=True)

        flat = work / "flat"
        component = flat / "base.pkg"
        component.mkdir(parents=True)
        paths = [Path("."), *sorted(path.relative_to(payload_root) for path in payload_root.rglob("*"))]
        size = math.ceil(sum(path.stat().st_size for path in payload_root.rglob("*") if path.is_file()) / 1024)
        with (work / "Payload.cpio").open("wb") as handle:
            subprocess.run(
                ["cpio", "--create", "--format=odc", "--owner=0:80", "--null"],
                cwd=payload_root,
                input=b"\0".join(str(path).encode() for path in paths) + b"\0",
                stdout=handle,
                check=True,
            )
        with (work / "Payload.cpio").open("rb") as source, (component / "Payload").open("wb") as destination:
            with gzip.GzipFile(filename="", mode="wb", fileobj=destination, mtime=0) as compressed:
                shutil.copyfileobj(source, compressed)
        subprocess.run(["mkbom", "-u", "0", "-g", "80", str(payload_root), str(component / "Bom")], check=True)

        package_id = f"{identifier}.installer"
        info = ET.Element("pkg-info", {
            "format-version": "2", "identifier": package_id, "version": version,
            "install-location": "/", "auth": "root",
        })
        ET.SubElement(info, "payload", {"installKBytes": str(size), "numberOfFiles": str(len(paths))})
        versions = ET.SubElement(info, "bundle-version")
        ET.SubElement(versions, "bundle", {
            "id": identifier, "CFBundleIdentifier": identifier,
            "path": f"./Applications/{product}.app", "CFBundleVersion": version,
        })
        write_xml(component / "PackageInfo", info)

        distribution = ET.Element("installer-gui-script", {"minSpecVersion": "2"})
        ET.SubElement(distribution, "title").text = product
        ET.SubElement(distribution, "options", {
            "customize": "never", "require-scripts": "false", "hostArchitectures": "arm64",
        })
        ET.SubElement(distribution, "domains", {
            "enable_anywhere": "false", "enable_currentUserHome": "false", "enable_localSystem": "true",
        })
        allowed = ET.SubElement(distribution, "allowed-os-versions")
        ET.SubElement(allowed, "os-version", {"min": minimum})
        outline = ET.SubElement(distribution, "choices-outline")
        ET.SubElement(outline, "line", {"choice": "default"})
        choice = ET.SubElement(distribution, "choice", {"id": "default", "visible": "false", "title": product})
        ET.SubElement(choice, "pkg-ref", {"id": package_id})
        ET.SubElement(distribution, "pkg-ref", {
            "id": package_id, "version": version, "installKBytes": str(size), "auth": "Root",
        }).text = "#base.pkg"
        write_xml(flat / "Distribution", distribution)

        installer = output / f"{product}_{version}_aarch64.pkg"
        subprocess.run([
            "bsdtar", "--format=xar", "--options=xar:compression=none",
            "-cf", str(installer), "-C", str(flat), "Distribution", "base.pkg",
        ], check=True)
        print(installer)


if __name__ == "__main__":
    main()
