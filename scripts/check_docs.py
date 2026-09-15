#!/usr/bin/env python3
"""Check the documentation and shared poem explorer, with optional cross-root comparison."""

import argparse
import hashlib
from html.parser import HTMLParser
import json
from pathlib import Path
import re
import tomllib
from urllib.parse import quote, unquote, urljoin, urlsplit
import xml.etree.ElementTree as ET

from project_logo import projection


REPOSITORY = Path(__file__).resolve().parents[1]
SITEMAP_NS = "{http://www.sitemaps.org/schemas/sitemap/0.9}"
XHTML_NS = "{http://www.w3.org/1999/xhtml}"
POEM_ROUTES = {
    "guide/poem/index.html": ("en", {"en": "guide/poem/", "de": "guide/poem/de/"}),
    "guide/poem/de/index.html": ("de", {"en": "guide/poem/", "de": "guide/poem/de/"}),
}
POEM_HOSTS = {"index.html"}
VOID_ELEMENTS = {"area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"}
CSS_URL = re.compile(
    r"url\(\s*(?:\"((?:\\.|[^\"\\])*)\"|'((?:\\.|[^'\\])*)'|([^)]*?))\s*\)",
    re.IGNORECASE,
)
CSS_IMPORT = re.compile(r"@import\s+(?:\"((?:\\.|[^\"\\])*)\"|'((?:\\.|[^'\\])*)')", re.IGNORECASE)
CSS_ESCAPE = re.compile(r"\\(?:([0-9a-fA-F]{1,6})\s?|([^\r\n]))")


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def origin(url):
    return (url.scheme.lower(), url.hostname, url.port or (443 if url.scheme == "https" else 80))


def deployment_url(value):
    parsed = urlsplit(value)
    require(parsed.scheme in {"https", "http"} and parsed.hostname, "base URL must be HTTP(S)")
    require(not parsed.username and not parsed.password and not any(char in value for char in "@?#"),
            "base URL must not contain credentials, query or fragment")
    path = parsed.path.removesuffix("/")
    require(not path or re.fullmatch(r"(?:/[A-Za-z0-9._-]+)+", path), "invalid base URL project path")
    require(all(segment not in {".", ".."} for segment in path.split("/")), "base URL contains dot segments")
    require("\\" not in value and value == value.strip(), "invalid base URL spelling")
    host = parsed.hostname.encode("idna").decode("ascii").lower()
    if ":" in host:
        host = f"[{host}]"
    port = parsed.port
    if port is not None and port != (443 if parsed.scheme == "https" else 80):
        host += f":{port}"
    return parsed._replace(netloc=host, path=path + "/").geturl()


def snapshot(root):
    require(root.is_dir() and not root.is_symlink(), f"not a regular output directory: {root}")
    files = {}
    for path in sorted(root.rglob("*")):
        name = path.relative_to(root).as_posix()
        require(not path.is_symlink(), f"generated symlink: {name}")
        if path.is_dir():
            continue
        require(path.is_file(), f"generated special file: {name}")
        data = path.read_bytes()
        files[name] = {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}
    return files


def css_urls(text):
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.DOTALL)
    for expression in (CSS_URL, CSS_IMPORT):
        for match in expression.finditer(text):
            value = next(group for group in match.groups() if group is not None).strip()
            yield CSS_ESCAPE.sub(lambda escape: chr(int(escape[1], 16)) if escape[1] else escape[2], value)


class Document(HTMLParser):
    def __init__(self, text, name):
        super().__init__(convert_charrefs=True)
        self.name = name
        self.ids = set()
        self.references = []
        self.head_links = []
        self.anchors = []
        self.language = None
        self.h1 = []
        self.images = []
        self.diagrams = []
        self.in_style = False
        self.style_text = []
        self.frames = []
        self.poem_sources = []
        self.poem_controls = []
        self.poem_elements = []
        self.poem_stack = []
        self.current_poem_source = None
        self.poem_link_open = False
        self.element_order = 0
        self.feed(text)
        self.close()
        for diagram in self.diagrams:
            if diagram.get("aria-hidden") == "true":
                continue
            labels = diagram.get("aria-labelledby", "").split()
            require(diagram.get("role") in {"img", "group"} and len(labels) >= 2 and set(labels) <= self.ids,
                    f"diagram needs linked title and description: {self.name}")

    def handle_starttag(self, tag, pairs):
        attrs = dict(pairs)
        self.element_order += 1
        if self.poem_stack or (tag == "fieldset" and "poem-workflow" in attrs.get("class", "").split()):
            element = {"tag": tag, "attrs": attrs, "order": self.element_order,
                       "end": self.element_order, "text": "", "parent": self.poem_stack[-1] if self.poem_stack else None}
            self.poem_elements.append(element)
            if tag not in VOID_ELEMENTS:
                self.poem_stack.append(element)
        require(tag not in {"script", "object", "embed", "base"}, f"unexpected active element {tag}: {self.name}")
        if tag == "iframe":
            require(self.name in POEM_HOSTS
                    and set(attrs) <= {"src", "title", "sandbox", "loading", "referrerpolicy"}
                    and attrs.get("sandbox") == "allow-same-origin"
                    and attrs.get("title", "").strip(),
                    f"only named poem frames on explorer pages with a same-origin-only sandbox are allowed: {self.name}")
            self.frames.append((self.element_order, attrs))
        if tag == "input" and attrs.get("name") in {"poem-language", "poem-stage"}:
            self.poem_controls.append(attrs)
        if tag == "p" and "poem-source" in attrs.get("class", "").split():
            self.current_poem_source = {"order": self.element_order, "links": [], "text": ""}
        if tag == "a" and self.current_poem_source is not None:
            self.current_poem_source["links"].append(attrs.get("href"))
            self.poem_link_open = True
        require(not any(key.startswith("on") for key in attrs), f"script event handler: {self.name}")
        require(not (tag == "meta" and attrs.get("http-equiv", "").lower() == "refresh"), f"refresh dependency: {self.name}")
        identifier = attrs.get("id")
        if identifier is not None:
            require(identifier and identifier not in self.ids, f"empty/duplicate id {identifier!r}: {self.name}")
            self.ids.add(identifier)
        if tag == "a" and attrs.get("name"):
            self.ids.add(attrs["name"])
        if tag == "html":
            self.language = attrs.get("lang")
        if tag == "h1":
            self.h1.append(identifier)
        if tag == "img":
            self.images.append(attrs)
        if tag == "svg":
            self.diagrams.append(attrs)
        if tag == "link":
            self.head_links.append(attrs)
        if tag == "a":
            self.anchors.append(attrs)
        if tag == "style":
            self.in_style = True
        for attribute in ("href", "src", "poster", "action", "cite", "xlink:href"):
            if attribute in attrs:
                value = attrs[attribute]
                require(value is not None, f"URL attribute without a value: {self.name}")
                resource = attribute in {"src", "poster", "xlink:href"} or tag in {"image", "use"}
                if tag == "link":
                    resource = not set(attrs.get("rel", "").split()).issubset({"canonical", "alternate"})
                self.references.append((value, resource))
        if "srcset" in attrs:
            for candidate in attrs["srcset"].split(","):
                fields = candidate.split()
                require(fields, f"empty srcset candidate: {self.name}")
                self.references.append((fields[0], True))
        for value in css_urls(attrs.get("style", "")):
            self.references.append((value, True))

    def handle_startendtag(self, tag, attrs):
        self.handle_starttag(tag, attrs)
        self.handle_endtag(tag)

    def handle_endtag(self, tag):
        if self.poem_stack and self.poem_stack[-1]["tag"] == tag:
            self.poem_stack.pop()["end"] = self.element_order
        if tag == "style":
            self.in_style = False
            self.references.extend((value, True) for value in css_urls("".join(self.style_text)))
            self.style_text.clear()
        if tag == "a":
            self.poem_link_open = False
        if tag == "p" and self.current_poem_source is not None:
            self.poem_sources.append(self.current_poem_source)
            self.current_poem_source = None

    def handle_data(self, data):
        for element in self.poem_stack:
            element["text"] += data
        if self.current_poem_source is not None and self.poem_link_open:
            self.current_poem_source["text"] += data
        if self.in_style:
            self.style_text.append(data)


class Site:
    def __init__(self, root, base_url):
        self.root = root
        self.base_url = base_url
        self.base = urlsplit(base_url)
        self.files = snapshot(root)
        self.documents = {
            name: Document((root / name).read_text(encoding="utf-8"), name)
            for name in self.files if name.endswith(".html")
        }
        self.svg_ids = {}
        self.checked_links = 0

    def url(self, name):
        route = name.removesuffix("index.html") if name.endswith("/index.html") or name == "index.html" else name
        return urljoin(self.base_url, quote(route, safe="/"))

    def local_link(self, source, destination, resource=False):
        require(not re.search(r"[\x00-\x20\\]", destination), f"invalid URL in {source}: {destination!r}")
        resolved = urlsplit(urljoin(self.url(source), destination))
        if resolved.scheme not in {"http", "https"}:
            require(not resource and resolved.scheme in {"mailto", "tel"}, f"unexpected URL scheme in {source}: {destination}")
            return
        if origin(resolved) != origin(self.base):
            require(not resource, f"external resource dependency in {source}: {destination}")
            return
        require(not resolved.username and not resolved.password, f"credential-bearing URL in {source}")
        path = unquote(resolved.path, errors="strict")
        prefix = self.base.path
        require(path == prefix.removesuffix("/") or path.startswith(prefix),
                f"local link escapes deployment prefix in {source}: {destination}")
        relative = path[len(prefix):] if path.startswith(prefix) else ""
        require(not any(part in {".", ".."} for part in relative.split("/")) and "\\" not in relative,
                f"unsafe local path in {source}: {destination}")
        if not relative or relative.endswith("/"):
            relative += "index.html"
        elif relative + "/index.html" in self.files:
            relative += "/index.html"
        require(relative in self.files, f"broken local link in {source}: {destination} -> {relative}")
        fragment = unquote(resolved.fragment, errors="strict")
        if fragment:
            if relative in self.documents:
                identifiers = self.documents[relative].ids
            elif relative in self.svg_ids:
                identifiers = self.svg_ids[relative]
            else:
                raise RuntimeError(f"cannot resolve fragment in {source}: {destination}")
            require(fragment in identifiers, f"broken fragment in {source}: {destination}")
        self.checked_links += 1

    def check_manifests(self, version):
        manifests = {name for name in self.files if name.endswith("regen-manifest.json")}
        require(manifests == {"regen-manifest.json", "guide/poem/regen-manifest.json"}, "missing or unexpected output manifest")
        for name in sorted(manifests):
            prefix = name.removesuffix("regen-manifest.json")
            expected = {path[len(prefix):]: digest for path, digest in self.files.items()
                        if path.startswith(prefix) and path != name}
            manifest = json.loads((self.root / name).read_text(encoding="utf-8"))
            require(manifest == {"format": 1, "generator": "ReGen", "version": version, "files": expected},
                    f"manifest metadata, complete inventory or byte hashes differ: {name}")

    def check_resources(self):
        svg_references = []
        for name in self.files:
            if name.endswith(".svg"):
                root = ET.fromstring((self.root / name).read_bytes())
                identifiers = set()
                for element in root.iter():
                    tag = element.tag.rsplit("}", 1)[-1].lower()
                    require(tag not in {"script", "foreignobject"}, f"active SVG content: {name}")
                    if "id" in element.attrib:
                        identifier = element.attrib["id"]
                        require(identifier and identifier not in identifiers, f"duplicate SVG id: {name}")
                        identifiers.add(identifier)
                    for key, value in element.attrib.items():
                        attribute = key.rsplit("}", 1)[-1].lower()
                        require(not attribute.startswith("on"), f"SVG event handler: {name}")
                        if attribute == "href":
                            svg_references.append((name, value))
                        svg_references.extend((name, destination) for destination in css_urls(value))
                    if tag == "style":
                        svg_references.extend((name, value) for value in css_urls("".join(element.itertext())))
                self.svg_ids[name] = identifiers
        for name, destination in svg_references:
            self.local_link(name, destination, resource=True)
        for name, document in self.documents.items():
            for destination, resource in document.references:
                self.local_link(name, destination, resource)
        for name in self.files:
            require(not name.endswith((".js", ".mjs", ".cjs", ".wasm")), f"unexpected script dependency: {name}")
            if name.endswith(".css"):
                for destination in css_urls((self.root / name).read_text(encoding="utf-8")):
                    self.local_link(name, destination, resource=True)

    def check_page(self, name, language, alternates, stylesheet):
        document = self.documents[name]
        require(document.language == language, f"incorrect page language: {name}")
        require(len(document.h1) == 1, f"expected one page heading: {name}")
        canonical = [link.get("href") for link in document.head_links if "canonical" in link.get("rel", "").split()]
        require(canonical == [self.url(name)], f"incorrect canonical URL: {name}")
        if alternates is not None:
            actual = [(link.get("hreflang"), link.get("href")) for link in document.head_links
                      if "alternate" in link.get("rel", "").split()]
            require(len(actual) == len(alternates) and dict(actual) == alternates, f"incorrect language alternates: {name}")
        stylesheets = [link.get("href", "") for link in document.head_links if "stylesheet" in link.get("rel", "").split()]
        require(len(stylesheets) == 1 and re.fullmatch(stylesheet, stylesheets[0]), f"incorrect versioned stylesheet: {name}")

    def check_sitemap(self, name, pages):
        root = ET.fromstring((self.root / name).read_bytes())
        require(root.tag == SITEMAP_NS + "urlset", f"invalid sitemap root: {name}")
        records = {}
        for entry in root.findall(SITEMAP_NS + "url"):
            location = entry.findtext(SITEMAP_NS + "loc")
            require(location not in records and location is not None, f"duplicate/missing sitemap location: {name}")
            self.local_link(name, location)
            alternate_entries = [(link.get("hreflang"), link.get("href")) for link in entry.findall(XHTML_NS + "link")]
            require(len(alternate_entries) == len(dict(alternate_entries)), f"duplicate sitemap alternate: {name}")
            for _, destination in alternate_entries:
                require(destination is not None, f"missing sitemap alternate URL: {name}")
                self.local_link(name, destination)
            records[location] = dict(alternate_entries)
        require(records == pages, f"sitemap routes/alternates do not match rendered pages: {name}")

    def check_poem_explorer(self, name):
        document = self.documents[name]
        elements = document.poem_elements

        def nodes(tag=None, css_class=None, within=None):
            return [node for node in elements
                    if (tag is None or node["tag"] == tag)
                    and (css_class is None or css_class in node["attrs"].get("class", "").split())
                    and (within is None or within["order"] < node["order"] <= within["end"])]

        def one(tag, css_class, within=None):
            matches = nodes(tag, css_class, within)
            require(len(matches) == 1, f"expected one {css_class} {tag}: {name}")
            return matches[0]

        workflow = one("fieldset", "poem-workflow")
        example = one("fieldset", "poem-example", workflow)
        for group in (example, workflow):
            legends = [node for node in nodes("legend", within=group) if node["parent"] is group]
            require(len(legends) == 1 and legends[0]["text"].strip()
                    and "disabled" not in group["attrs"], f"poem control group needs a name and must be enabled: {name}")

        def controls(group, control_name, values, selected):
            radios = [node for node in nodes("input") if node["attrs"].get("name") == control_name]
            require([node["attrs"] for node in radios]
                    == [attrs for attrs in document.poem_controls if attrs.get("name") == control_name],
                    f"{control_name} choices must belong to the explorer: {name}")
            require([node["attrs"].get("value") for node in radios] == values,
                    f"incorrect {control_name} choices: {name}")
            require(["checked" in node["attrs"] for node in radios] == [value == selected for value in values],
                    f"incorrect initial {control_name} selection: {name}")
            for node, value in zip(radios, values):
                attrs = node["attrs"]
                require(attrs.get("type") == "radio" and attrs.get("id")
                        and group["order"] < node["order"] <= group["end"] and "disabled" not in attrs
                        and "hidden" not in attrs and attrs.get("tabindex", "0") == "0",
                        f"{control_name} must use enabled keyboard-selectable native radios: {name}")
                labels = [label for label in nodes("label") if label["attrs"].get("for") == attrs["id"]]
                require(len(labels) == 1 and labels[0]["text"].strip(),
                        f"{control_name} needs a meaningful associated label: {name}")
                label = labels[0]
                if control_name == "poem-language":
                    require(label["attrs"].get("id") == f"poem-tab-{value}",
                            f"poem language label needs its accessible panel reference: {name}")
            return radios

        languages = controls(example, "poem-language", ["en", "de"], "en")
        stages = controls(workflow, "poem-stage", ["source", "build", "site"], "source")
        panels = one("div", "poem-workflow-panels", workflow)
        stage_panels = nodes("section", "poem-workflow-panel", panels)
        require(len(stage_panels) == 3, f"expected three poem stage panels: {name}")
        source, build, site = [one("section", f"poem-workflow-{stage}", panels) for stage in ("source", "build", "site")]
        require(stage_panels == [source, build, site],
                f"poem panels must follow Source, Build, Site order: {name}")
        language_panels = {}
        for stage in (source, site):
            children = nodes(css_class="poem-panel", within=stage)
            require(len(children) == 2, f"source and site must each provide two poem languages: {name}")
            for child, language in zip(children, ("en", "de")):
                require(f"poem-panel-{language}" in child["attrs"].get("class", "").split()
                        and child["attrs"].get("lang") == language
                        and (stage is source
                             or f"poem-tab-{language}" in child["attrs"].get("aria-labelledby", "").split()),
                        f"poem language panel must identify its language: {name}")
                language_panels[(stage["order"], language)] = child
                if stage is source:
                    code = one("pre", "poem-code", child)
                    require(re.search(r"(?m)^data:\s*$", code["text"])
                            and re.search(r"(?m)^\s+lines:\s*$", code["text"])
                            and re.search(r"(?m)^\s+-\s+\S", code["text"]),
                            f"source stage must show a YAML verse excerpt: {name}")
                    input_url = f"https://github.com/CritX-ai/ReGen/blob/main/examples/minimal/content/{language}/pages/index.yaml"
                    require(any(node["attrs"].get("href") == input_url
                                and node["text"].strip() for node in nodes("a", within=child)),
                            f"source excerpt must link its complete public input: {name}")
        command = one("pre", "poem-command", build)
        require(" ".join(command["text"].split()) == "regen build --site examples/minimal",
                f"build stage must show the real example command: {name}")
        outputs = one(None, "poem-output-files", build)
        require(all(path in outputs["text"] for path in ("dist/index.html", "dist/de/index.html")),
                f"build stage must name both independent output files: {name}")
        require(not nodes("button", within=build) and not nodes("input", within=build),
                f"build stage must not pretend to execute in the browser: {name}")
        poem_urls = [self.base_url + path for path in ("guide/poem/", "guide/poem/de/")]
        require([frame.get("src") for _, frame in document.frames] == poem_urls,
                f"explorer must embed exactly the two independently generated poem URLs: {name}")
        require(len(document.poem_sources) == 2, f"missing embedded poem URL captions: {name}")
        for (frame_order, _), caption, url, language in zip(
                document.frames, document.poem_sources, poem_urls, ("en", "de")):
            panel = language_panels[(site["order"], language)]
            require(panel["order"] < frame_order < caption["order"] <= panel["end"]
                    and caption["links"] == [url] and caption["text"].strip() == url,
                    f"each site poem must precede its matching visible URL in its language panel: {name}")

    def check(self, docs, version):
        self.check_manifests(version)
        badge = ET.fromstring((self.root / "brand/version.svg").read_bytes())
        visible_versions = [node.text for node in badge.iter("{http://www.w3.org/2000/svg}text")]
        require(version in visible_versions, "visible source-version badge differs from the generator version")
        animated = (self.root / "brand/regen-logo.svg").read_text(encoding="utf-8")
        static = (self.root / "brand/regen-logo-static.svg").read_text(encoding="utf-8")
        require(projection(animated) == static, "static wordmark differs from the animated logo's default artwork")
        expected_docs = {(page["slug"] + "/" if page["slug"] else "") + "index.html" for page in docs}
        require(set(self.documents) == expected_docs | set(POEM_ROUTES), "unexpected or missing documentation/poem page")
        self.check_resources()
        require("build-topology" in self.documents["index.html"].ids,
                "homepage is missing its pipeline overview")
        wordmarks = {self.base_url + "brand/" + name for name in ("regen-logo.svg", "regen-logo-static.svg")}
        require(sum(urljoin(self.url("index.html"), image.get("src", "")) in wordmarks
                    for image in self.documents["index.html"].images) == 1,
                "homepage must have one wordmark, not a repeated README introduction")
        prefix = re.escape(self.base.path)
        docs_urls = {self.url(name) for name in expected_docs}
        docs_sitemap = {}
        for name in sorted(expected_docs):
            self.check_page(name, "en", None, prefix + r"assets/[0-9a-f]{64}/docs\.css")
            document = self.documents[name]
            require(document.h1[0] is not None, f"documentation title lost its source anchor: {name}")
            navigation = {urljoin(self.url(name), link.get("href", "")) for link in document.anchors}
            require(docs_urls <= navigation, f"incomplete documentation navigation: {name}")
            docs_sitemap[self.url(name)] = {"en": self.url(name)}
        for name in sorted(POEM_HOSTS):
            self.check_poem_explorer(name)
        require(all(not document.poem_elements for name, document in self.documents.items() if name not in POEM_HOSTS),
                "poem explorer is only allowed on homepage and guide")
        poem_sitemap = {}
        for name, (language, paths) in POEM_ROUTES.items():
            alternates = {code: self.base_url + path for code, path in paths.items()}
            self.check_page(name, language, alternates, prefix + r"guide/poem/assets/[0-9a-f]{64}/site\.css")
            poem_sitemap[self.url(name)] = alternates
        self.check_sitemap("sitemap.xml", docs_sitemap)
        self.check_sitemap("guide/poem/sitemap.xml", poem_sitemap)
        for name, expected in {
            "robots.txt": {self.base_url + "sitemap.xml", self.base_url + "guide/poem/sitemap.xml"},
            "guide/poem/robots.txt": {self.base_url + "guide/poem/sitemap.xml"},
        }.items():
            locations = [line.split(":", 1)[1].strip() for line in (self.root / name).read_text(encoding="utf-8").splitlines()
                         if line.lower().startswith("sitemap:")]
            require(len(locations) == len(expected) and set(locations) == expected, f"incorrect robots sitemap URLs: {name}")
            for location in locations:
                self.local_link(name, location)
        return self.files


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--site", required=True, type=Path, help="Completed static output directory")
    parser.add_argument("--base-url", required=True, help="Same deployment URL passed to build_docs")
    parser.add_argument("--compare", type=Path, help="Independently built output; check it and compare every path/byte")
    args = parser.parse_args()
    base_url = deployment_url(args.base_url)
    docs = json.loads((REPOSITORY / "site/pages.json").read_text(encoding="utf-8"))
    version = tomllib.loads((REPOSITORY / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
    site = Site(args.site, base_url)
    files = site.check(docs, version)
    print(f"Documentation and guide poem: {len(site.documents)} pages, {len(files)} files, {site.checked_links} local references; both manifests verified")
    if args.compare is not None:
        other = Site(args.compare, base_url)
        require(other.check(docs, version) == files, "output inventories or hashes differ across build roots")
        for name in files:
            require((args.site / name).read_bytes() == (args.compare / name).read_bytes(), f"output bytes differ across build roots: {name}")
        print(f"Independent output roots have identical paths and bytes: {len(files)} files")


if __name__ == "__main__":
    main()
