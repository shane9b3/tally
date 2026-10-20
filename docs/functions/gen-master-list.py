#!/usr/bin/env python3
"""Generate docs/functions/master-list.toml from docs/calc-functions.json.

Run from anywhere:  python3 -I docs/functions/gen-master-list.py

- One [[function]] per AOO entry, in JSON order, then the extensions
  (IFERROR, IFNA, ISOWEEKNUM) that AOO 4.1 lacks.
- `batch = 0` marks the 68 batch-0 names, read from the
  "## Batch-0 functions" section of docs/phase-a.md (bullet lines only).
- Lead-approved corrections live in the explicit override tables below
  (ARITY_OVERRIDES, ODF_OVERRIDES, EXTENSION_OVERRIDES, OOXML_NAMES); each
  override is checked against the JSON so a stale entry fails loudly.
- `xtask function-coverage` and the tally-functions test
  `registry_matches_master_list` compare this file with the registry.
"""
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
SRC = ROOT / "docs" / "calc-functions.json"
PHASE_A = ROOT / "docs" / "phase-a.md"
OUT = ROOT / "docs" / "functions" / "master-list.toml"

# Functions Tally adds on top of AOO 4.1 (Excel/LibreOffice compatible).
EXTENSIONS = [
    {"name": "IFERROR", "category": "Logical", "odfName": "COM.MICROSOFT.IFERROR",
     "minArgs": 2, "maxArgs": 2, "source": "extension"},
    {"name": "IFNA", "category": "Logical", "odfName": "COM.MICROSOFT.IFNA",
     "ooxmlName": "_xlfn.IFNA", "minArgs": 2, "maxArgs": 2, "source": "extension"},
    # Excel 2013 ISOWEEKNUM(date). Plain ODF ISOWEEKNUM is AOO's built-in
    # WEEKNUM, so the extension is stored as COM.MICROSOFT.ISOWEEKNUM; the
    # optional second argument keeps a 2-argument ODF ISOWEEKNUM resolved by
    # its canonical name evaluating as AOO's WEEKNUM (LibreOffice tdf#50950).
    {"name": "ISOWEEKNUM", "category": "Date&Time",
     "odfName": "COM.MICROSOFT.ISOWEEKNUM", "ooxmlName": "_xlfn.ISOWEEKNUM",
     "minArgs": 1, "maxArgs": 2, "source": "extension",
     "note": "Excel ISOWEEKNUM; ODF ISOWEEKNUM is the built-in WEEKNUM"},
]
BATCH0_COUNT = 68

# Lead-approved arity corrections to the AOO wizard data.
# TRUNC: AOO's wizard lists `count` as required, but ODFF 1.2 and Excel make
# it optional (default 0); correctness review 2026-10-19.
ARITY_OVERRIDES = {
    "TRUNC": {"minArgs": 1},
    # NUMBERVALUE: AOO declares it (2 args) but never implemented it; Tally
    # implements the ODFF 1.2 / Excel 2013 form NUMBERVALUE(text; [decimal_sep];
    # [group_sep]) as an extension (see EXTENSION_OVERRIDES).
    "NUMBERVALUE": {"minArgs": 1, "maxArgs": 3},
    # SUBTOTAL: the wizard lists one range, but ScSubTotal only checks
    # MustHaveParamCountMin(2) and the chosen function takes every further
    # reference (correctness review 3, M2).
    "SUBTOTAL": {"maxArgs": 255},
}

# ODF-name clashes between the Analysis add-in and built-ins (AOO rule,
# docs/phase-a.md): plain ISEVEN/ISODD/GCD/LCM/EFFECT/CUMPRINC/CUMIPMT/NOMINAL
# mean the BUILT-IN (odffmap.cxx bMapDupToInternal=true), so the add-in loses
# the plain name and is stored under its UNO programmatic name; plain
# WEEKNUM/CONVERT/DURATION mean the ADD-IN (bMapDupToInternal=false), so the
# built-in keeps its own ODF name (ISOWEEKNUM / ORG.OPENOFFICE.CONVERT /
# PDURATION, already in the JSON). Values marked `None` are filled from the
# JSON `programmaticName`; explicit strings must differ from the JSON value.
ADDIN_LOSES_PLAIN_NAME = [
    "ISEVEN_ADD", "ISODD_ADD", "GCD_ADD", "LCM_ADD",
    "EFFECT_ADD", "CUMPRINC_ADD", "CUMIPMT_ADD", "NOMINAL_ADD",
]
ADDIN_KEEPS_PLAIN_NAME = {
    # add-in -> (plain ODF name, built-in that must NOT use it)
    "WEEKNUM_ADD": ("WEEKNUM", "WEEKNUM"),
    "CONVERT_ADD": ("CONVERT", "CONVERT"),
    "DURATION_ADD": ("DURATION", "DURATION"),
}
ODF_OVERRIDES = {
    # ZGZ is AOO's (German) name for ODFF RRI; core_resource.src stores RRI.
    "ZGZ": "RRI",
}

# Not implemented by AOO 4.1 (so `extension = true`) although AOO lists it.
EXTENSION_OVERRIDES = {"NUMBERVALUE"}

# OOXML (Excel) names. Set when Excel has the function and either (a) the
# Excel name differs from ours, (b) Excel 2010+ stores it with the `_xlfn.`
# prefix, or (c) it is an Analysis add-in function (AOO maps add-ins through
# a separate table on .xlsx import, so the mapping is spelled out). Built-ins
# whose Excel name equals the canonical name need no entry: the registry's
# OOXML lookup falls back to canonical names.
XLFN_2010_PLUS = [
    "ACOT", "ACOTH", "ARABIC", "BASE", "BITAND", "BITLSHIFT", "BITOR",
    "BITRSHIFT", "BITXOR", "COMBINA", "COT", "COTH", "CSC", "CSCH", "DAYS",
    "DECIMAL", "GAMMA", "GAUSS", "IFNA", "IMCOSH", "IMCOT", "IMCSC", "IMCSCH",
    "IMSEC", "IMSECH", "IMSINH", "IMTAN", "ISFORMULA", "MUNIT", "NUMBERVALUE",
    "PERMUTATIONA", "PHI", "SEC", "SECH", "SHEET", "SHEETS", "UNICHAR",
    "UNICODE", "XOR",
]
OOXML_RENAMED = {
    "FORMULA": "_xlfn.FORMULATEXT",
    "EFFECTIVE": "EFFECT",
    "DURATION": "_xlfn.PDURATION",
    "ZGZ": "_xlfn.RRI",
    # WEEKNUM (built-in) has no OOXML name: AOO's 4-day week rule matches
    # neither Excel WEEKNUM (week 1 contains 1 January; that is WEEKNUM_ADD)
    # nor Excel's 1-argument ISOWEEKNUM (the ISOWEEKNUM extension). It is
    # exported under its canonical name (correctness review 2, H2/H6).
    "WEEKNUM_ADD": "WEEKNUM",
    "CONVERT_ADD": "CONVERT",
    "DURATION_ADD": "DURATION",
}


def ooxml_names(funcs):
    out = {}
    names = {f["name"] for f in funcs}
    for n in XLFN_2010_PLUS:
        if n not in names:
            sys.exit(f"XLFN_2010_PLUS: unknown function {n}")
        out[n] = "_xlfn." + n
    for f in funcs:
        if f["source"] == "analysis-addin" and f["name"] not in ADDIN_LOSES_PLAIN_NAME:
            out.setdefault(f["name"], f["name"])
    for n, x in OOXML_RENAMED.items():
        if n not in names:
            sys.exit(f"OOXML_RENAMED: unknown function {n}")
        out[n] = x
    if len(set(out.values())) != len(out):
        sys.exit("OOXML names are not unique")
    return out


def apply_odf_overrides(funcs):
    by = {f["name"]: f for f in funcs}
    for n in ADDIN_LOSES_PLAIN_NAME:
        f = by[n]
        if not f.get("odfMapDupToInternal"):
            sys.exit(f"{n}: JSON says the plain name maps to the add-in")
        f["odfName"] = f["programmaticName"].upper()
        f["note"] = (f"plain ODF name {n[:-4]} means the built-in (AOO rule); "
                     "stored under the add-in programmatic name")
    for n, (plain, builtin) in ADDIN_KEEPS_PLAIN_NAME.items():
        if by[n]["odfName"] != plain or by[n].get("odfMapDupToInternal"):
            sys.exit(f"{n}: expected the add-in to own ODF name {plain}")
        if by[builtin]["odfName"] == plain:
            sys.exit(f"{builtin}: built-in must not use ODF name {plain}")
    for n, odf in ODF_OVERRIDES.items():
        if by[n]["odfName"] == odf:
            sys.exit(f"ODF_OVERRIDES[{n}] is redundant")
        by[n]["odfName"] = odf
        by[n]["note"] = f"ODF (OpenFormula) name is {odf}"
    odf = [f["odfName"].upper() for f in funcs]
    dups = sorted({o for o in odf if odf.count(o) > 1})
    if dups:
        sys.exit(f"duplicate ODF names after overrides: {dups}")


def toml_str(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def batch0_names(known):
    text = PHASE_A.read_text(encoding="utf-8")
    m = re.search(r"^## Batch-0 functions.*?$(.*?)(?=^## |\Z)", text, re.S | re.M)
    if not m:
        sys.exit("phase-a.md: no '## Batch-0 functions' section")
    names = set()
    for line in m.group(1).splitlines():
        if not line.startswith("- "):
            continue
        body = line.split(":", 1)[1] if ":" in line else line
        for tok in re.findall(r"\b[A-Z][A-Z0-9_.]*[A-Z0-9]\b", body):
            if tok in known:
                names.add(tok)
    if len(names) != BATCH0_COUNT:
        sys.exit(f"expected {BATCH0_COUNT} batch-0 names, found {len(names)}: {sorted(names)}")
    return names


def main():
    funcs = json.loads(SRC.read_text(encoding="utf-8"))
    names = [f["name"] for f in funcs]
    if len(set(names)) != len(names):
        sys.exit("duplicate names in calc-functions.json")
    all_funcs = funcs + EXTENSIONS
    apply_odf_overrides(all_funcs)
    ooxml = ooxml_names(all_funcs)
    for n in EXTENSION_OVERRIDES:
        next(f for f in all_funcs if f["name"] == n)["note"] = (
            "declared but never implemented in AOO 4.1; Tally implements the "
            "ODFF/Excel 3-argument form")
    batch0 = batch0_names({f["name"] for f in all_funcs})
    out = [
        "# GENERATED by docs/functions/gen-master-list.py from docs/calc-functions.json",
        "# and docs/phase-a.md. Do not edit by hand; rerun the script.",
        "#",
        "# category: AOO wizard category (Add-in functions keep \"Add-in\").",
        "# max_args = 255 means variadic. batch = 0 marks the Phase A batch.",
        "# extension = true: not in AOO 4.1 (Excel/LibreOffice compatible).",
        "# odf_name / ooxml_name / arity overrides: see the tables in the script.",
        "",
    ]
    for f in all_funcs:
        f = {**f, **ARITY_OVERRIDES.get(f["name"], {})}
        out.append("[[function]]")
        out.append(f"name = {toml_str(f['name'])}")
        out.append(f"category = {toml_str(f['category'])}")
        out.append(f"odf_name = {toml_str(f['odfName'])}")
        x = ooxml.get(f["name"]) or f.get("ooxmlName")
        if x:
            out.append(f"ooxml_name = {toml_str(x)}")
        out.append(f"min_args = {int(f['minArgs'])}")
        out.append(f"max_args = {int(f['maxArgs'])}")
        out.append(f"source = {toml_str(f['source'])}")
        if f["name"] in batch0:
            out.append("batch = 0")
        ext = f["source"] == "extension" or f["name"] in EXTENSION_OVERRIDES
        out.append(f"extension = {'true' if ext else 'false'}")
        if f.get("note"):
            out.append(f"note = {toml_str(f['note'])}")
        out.append("")
    OUT.write_text("\n".join(out), encoding="utf-8")
    print(f"wrote {OUT.relative_to(ROOT)}: {len(all_funcs)} functions, {len(batch0)} in batch 0")


if __name__ == "__main__":
    main()
