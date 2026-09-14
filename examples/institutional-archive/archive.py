#!/usr/bin/env python3
"""Small idempotent LaTeX Core institutional archive API example."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile
from urllib.error import HTTPError
from urllib.parse import urlencode, urljoin
from urllib.request import Request, urlopen


class Api:
    def __init__(self, base_url: str, token: str):
        self.base = base_url.rstrip("/") + "/"
        self.token = token

    def get(self, path: str):
        request = Request(urljoin(self.base, path.lstrip("/")))
        request.add_header("Authorization", "Bearer " + self.token)
        try:
            with urlopen(request, timeout=30) as response:
                return response.read(), {
                    key.lower(): value for key, value in response.headers.items()
                }
        except HTTPError as error:
            body = error.read()
            if error.code == 404:
                return None, {"error": body.decode("utf-8", "replace")}
            raise RuntimeError(f"API request failed with HTTP {error.code}") from error

    def json(self, path: str):
        body, _ = self.get(path)
        if body is None:
            return None
        return json.loads(body)

    def pages(self, path: str):
        cursor = None
        while True:
            separator = "&" if "?" in path else "?"
            query = path + separator + urlencode({"limit": 100, **({"cursor": cursor} if cursor else {})})
            page = self.json(query)
            if page is None:
                return
            yield from page["data"]
            cursor = page["page"]["next_cursor"]
            if not cursor:
                return


def atomic_write(path: Path, content: bytes):
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as handle:
        handle.write(content)
        temporary = Path(handle.name)
    temporary.replace(path)


def write_json(path: Path, value):
    atomic_write(path, (json.dumps(value, indent=2, sort_keys=True) + "\n").encode())


def safe_name(path: str):
    return path.replace("/", "__").replace("\\", "__")


def verified_download(api: Api, url: str, expected: str, target: Path):
    body, headers = api.get(url)
    if body is None:
        return False
    actual = hashlib.sha256(body).hexdigest()
    if actual != expected:
        raise RuntimeError(f"hash mismatch for {url}: expected {expected}, got {actual}")
    etag = headers.get("etag", "").strip('"')
    if etag and etag != actual:
        raise RuntimeError(f"ETag mismatch for {url}")
    atomic_write(target, body)
    return True


def archive(api: Api, output: Path):
    reports = list(api.pages("/api/integration/v1/reports"))
    write_json(output / "reports.json", reports)
    for report in reports:
        report_id = report["id"]
        root = output / "reports" / report_id
        detail = api.json(f"/api/integration/v1/reports/{report_id}")
        metadata = api.json(f"/api/integration/v1/reports/{report_id}/front-matter")
        versions = list(api.pages(f"/api/integration/v1/reports/{report_id}/versions"))
        write_json(root / "report.json", detail)
        write_json(root / "front-matter.json", metadata)
        write_json(root / "versions.json", versions)
        for version in versions:
            version_id = version["id"]
            manifest = api.json(f"/api/integration/v1/reports/{report_id}/versions/{version_id}/files")
            write_json(root / "versions" / version_id / "files.json", manifest)
            for item in manifest.get("files", []):
                verified_download(api, item["content_url"], item["sha256"], root / "versions" / version_id / "files" / safe_name(item["path"]))

        # Each download uses an explicit immutable build ID. `is_current` is
        # retained so a last-good PDF is never represented as today's report.
        builds = list(api.pages(f"/api/integration/v1/reports/{report_id}/builds"))
        write_json(root / "builds.json", builds)
        results = []
        for build in builds:
            build_id = build["id"]
            body, headers = api.get(build["download_url"])
            if body is None:
                results.append({"build_id": build_id, "status": "not_available"})
                continue
            expected = headers.get("etag", "").strip('"')
            actual = hashlib.sha256(body).hexdigest()
            if expected and expected != actual:
                raise RuntimeError(f"PDF hash mismatch for build {build_id}")
            atomic_write(root / "pdf" / f"{build_id}.pdf", body)
            results.append({
                "build_id": build_id,
                "status": "downloaded",
                "sha256": actual,
                "version_id": headers.get("x-latex-core-version-id"),
                "is_current": build["is_current"],
            })
        write_json(root / "pdf-status.json", results or [{"status": "missing_build_identity"}])


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--base-url", required=True)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    token = os.environ.get("LATEX_CORE_INTEGRATION_TOKEN")
    if not token:
        raise SystemExit("LATEX_CORE_INTEGRATION_TOKEN is required")
    archive(Api(args.base_url, token), args.output)


if __name__ == "__main__":
    main()
