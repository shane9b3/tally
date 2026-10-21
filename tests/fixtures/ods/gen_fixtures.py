#!/usr/bin/env python3
"""Generates the ODS/CSV/malicious test fixtures for tally-io-ods and tally-io-csv.

Run from anywhere with:  python3 -I tests/fixtures/ods/gen_fixtures.py

The ODS files reproduce the XML *shapes* LibreOffice 7.x and Apache
OpenOffice 4.1 write (repeated rows/columns, covered cells, text:p with
spans/text:s, of:= formulas with [.A1] references, number:* data styles with
style:map, settings.xml with ActiveTable, ...). They are hand-written, not
saved by those applications. Output is deterministic (fixed timestamps).
"""

import os
import struct
import zipfile
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
FIXTURES = os.path.dirname(HERE)
ODS_DIR = os.path.join(FIXTURES, "ods")
CSV_DIR = os.path.join(FIXTURES, "csv")
BAD_DIR = os.path.join(FIXTURES, "malicious")

MIMETYPE = b"application/vnd.oasis.opendocument.spreadsheet"
EPOCH = (1980, 1, 1, 0, 0, 0)

NS = (
    ' xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"'
    ' xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"'
    ' xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"'
    ' xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"'
    ' xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"'
    ' xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"'
    ' xmlns:xlink="http://www.w3.org/1999/xlink"'
    ' xmlns:dc="http://purl.org/dc/elements/1.1/"'
    ' xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0"'
    ' xmlns:number="urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0"'
    ' xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0"'
    ' xmlns:of="urn:oasis:names:tc:opendocument:xmlns:of:1.2"'
    ' xmlns:oooc="http://openoffice.org/2004/calc"'
    ' xmlns:config="urn:oasis:names:tc:opendocument:xmlns:config:1.0"'
    ' xmlns:tableooo="http://openoffice.org/2009/table"'
    ' xmlns:calcext="urn:org:documentfoundation:names:experimental:calc:xmlns:calcext:1.0"'
    ' xmlns:loext="urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0"'
)

DECL = '<?xml version="1.0" encoding="UTF-8"?>\n'


def info(name, method=zipfile.ZIP_DEFLATED):
    zi = zipfile.ZipInfo(name, date_time=EPOCH)
    zi.compress_type = method
    zi.external_attr = 0o644 << 16
    return zi


def write_zip(path, entries, mimetype_first=True, mimetype_method=zipfile.ZIP_STORED):
    """entries: list of (name, bytes|str[, method])."""
    with zipfile.ZipFile(path, "w") as z:
        if mimetype_first:
            z.writestr(info("mimetype", mimetype_method), MIMETYPE)
        for e in entries:
            name, data = e[0], e[1]
            method = e[2] if len(e) > 2 else zipfile.ZIP_DEFLATED
            if isinstance(data, str):
                data = data.encode("utf-8")
            z.writestr(info(name, method), data)
        if not mimetype_first:
            z.writestr(info("mimetype", mimetype_method), MIMETYPE)


def manifest(version="1.3", extra=()):
    files = ["content.xml", "styles.xml", "meta.xml", "settings.xml"] + list(extra)
    out = [DECL, '<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="%s">\n' % version]
    out.append(' <manifest:file-entry manifest:full-path="/" manifest:version="%s" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/>\n' % version)
    for f in files:
        out.append(' <manifest:file-entry manifest:full-path="%s" manifest:media-type="text/xml"/>\n' % f)
    out.append("</manifest:manifest>\n")
    return "".join(out)


def content_doc(body, auto_styles="", fonts="", version="1.3", calc=""):
    return (
        DECL
        + '<office:document-content%s office:version="%s">' % (NS, version)
        + "<office:scripts/>"
        + "<office:font-face-decls>%s</office:font-face-decls>" % fonts
        + "<office:automatic-styles>%s</office:automatic-styles>" % auto_styles
        + "<office:body><office:spreadsheet>%s%s</office:spreadsheet></office:body>" % (calc, body)
        + "</office:document-content>"
    )


def simple_table(name, rows_xml):
    return (
        '<table:table table:name="%s" table:style-name="ta1">' % name
        + '<table:table-column table:style-name="co1" table:number-columns-repeated="1024" table:default-cell-style-name="Default"/>'
        + rows_xml
        + "</table:table>"
    )


def str_cell(t):
    return '<table:table-cell office:value-type="string" calcext:value-type="string"><text:p>%s</text:p></table:table-cell>' % t


def float_cell(v, style=None):
    s = ' table:style-name="%s"' % style if style else ""
    return '<table:table-cell%s office:value-type="float" office:value="%s" calcext:value-type="float"><text:p>%s</text:p></table:table-cell>' % (s, v, v)


BASE_AUTO = (
    '<style:style style:name="co1" style:family="table-column"><style:table-column-properties fo:break-before="auto" style:column-width="2.258cm"/></style:style>'
    '<style:style style:name="ro1" style:family="table-row"><style:table-row-properties style:row-height="0.452cm" fo:break-before="auto" style:use-optimal-row-height="true"/></style:style>'
    '<style:style style:name="ta1" style:family="table" style:master-page-name="Default"><style:table-properties table:display="true" style:writing-mode="lr-tb"/></style:style>'
)


# ---------------------------------------------------------------------------
# LibreOffice 7.x shaped document
# ---------------------------------------------------------------------------

LO_FONTS = (
    '<style:font-face style:name="Liberation Sans" svg:font-family="&apos;Liberation Sans&apos;" style:font-family-generic="swiss" style:font-pitch="variable"/>'
    '<style:font-face style:name="DejaVu Sans" svg:font-family="&apos;DejaVu Sans&apos;" style:font-family-generic="system" style:font-pitch="variable"/>'
)

LO_AUTO = (
    '<style:style style:name="co1" style:family="table-column"><style:table-column-properties fo:break-before="auto" style:column-width="4.5cm"/></style:style>'
    '<style:style style:name="co2" style:family="table-column"><style:table-column-properties fo:break-before="auto" style:column-width="2.258cm"/></style:style>'
    '<style:style style:name="co3" style:family="table-column"><style:table-column-properties fo:break-before="auto" style:column-width="0.889in"/></style:style>'
    '<style:style style:name="ro1" style:family="table-row"><style:table-row-properties style:row-height="0.452cm" fo:break-before="auto" style:use-optimal-row-height="true"/></style:style>'
    '<style:style style:name="ro2" style:family="table-row"><style:table-row-properties style:row-height="1cm" fo:break-before="auto" style:use-optimal-row-height="false"/></style:style>'
    '<style:style style:name="ta1" style:family="table" style:master-page-name="Default"><style:table-properties table:display="true" style:writing-mode="lr-tb"/></style:style>'
    '<style:style style:name="ta2" style:family="table" style:master-page-name="Default"><style:table-properties table:display="true" style:writing-mode="lr-tb" tableooo:tab-color="#3366cc"/></style:style>'
    '<number:currency-style style:name="N104P0" style:volatile="true"><number:currency-symbol number:language="en" number:country="US">$</number:currency-symbol><number:number number:decimal-places="2" number:min-decimal-places="2" number:min-integer-digits="1" number:grouping="true"/></number:currency-style>'
    '<number:currency-style style:name="N104"><style:text-properties fo:color="#ff0000"/><number:text>-</number:text><number:currency-symbol number:language="en" number:country="US">$</number:currency-symbol><number:number number:decimal-places="2" number:min-decimal-places="2" number:min-integer-digits="1" number:grouping="true"/><style:map style:condition="value()&gt;=0" style:apply-style-name="N104P0"/></number:currency-style>'
    '<number:date-style style:name="N37" number:automatic-order="true"><number:month number:style="long"/><number:text>/</number:text><number:day number:style="long"/><number:text>/</number:text><number:year number:style="long"/></number:date-style>'
    '<number:percentage-style style:name="N11"><number:number number:decimal-places="2" number:min-decimal-places="2" number:min-integer-digits="1"/><number:text>%</number:text></number:percentage-style>'
    '<number:time-style style:name="N41"><number:hours number:style="long"/><number:text>:</number:text><number:minutes number:style="long"/><number:text> </number:text><number:am-pm/></number:time-style>'
    '<number:date-style style:name="N50"><number:day number:style="long"/><number:text> </number:text><number:month number:textual="true"/><number:text> </number:text><number:year number:style="long"/></number:date-style>'
    '<number:number-style style:name="N60"><number:scientific-number number:decimal-places="2" number:min-decimal-places="2" number:min-integer-digits="1" number:min-exponent-digits="2" number:exponent-interval="1" number:forced-exponent-sign="true"/></number:number-style>'
    '<number:number-style style:name="N70"><number:fraction number:min-integer-digits="0" number:min-numerator-digits="1" number:min-denominator-digits="1"/></number:number-style>'
    '<number:text-style style:name="N100"><number:text-content/></number:text-style>'
    '<number:number-style style:name="N80"><number:text>Qty </number:text><number:number number:decimal-places="0" number:min-decimal-places="0" number:min-integer-digits="1"/></number:number-style>'
    '<style:style style:name="ce1" style:family="table-cell" style:parent-style-name="Default"><style:text-properties fo:font-weight="bold" style:font-weight-asian="bold" style:font-weight-complex="bold"/></style:style>'
    '<style:style style:name="ce2" style:family="table-cell" style:parent-style-name="Default" style:data-style-name="N104"/>'
    '<style:style style:name="ce3" style:family="table-cell" style:parent-style-name="Default" style:data-style-name="N37"/>'
    '<style:style style:name="ce4" style:family="table-cell" style:parent-style-name="Default" style:data-style-name="N11"/>'
    '<style:style style:name="ce5" style:family="table-cell" style:parent-style-name="Default"><style:table-cell-properties fo:background-color="#ffff00" style:text-align-source="fix" style:repeat-content="false" fo:wrap-option="wrap" style:vertical-align="middle"/><style:paragraph-properties fo:text-align="center" fo:margin-left="0cm"/><style:text-properties fo:font-style="italic" fo:color="#c9211e" fo:font-size="14pt" style:font-name="DejaVu Sans" style:text-underline-style="solid" style:text-underline-width="auto" style:text-underline-color="font-color"/></style:style>'
    '<style:style style:name="ce6" style:family="table-cell" style:parent-style-name="Default" style:data-style-name="N41"/>'
    '<style:style style:name="ce7" style:family="table-cell" style:parent-style-name="Accent"><style:text-properties fo:font-style="italic"/></style:style>'
    '<style:style style:name="ce8" style:family="table-cell" style:parent-style-name="Default" style:data-style-name="N50"/>'
    '<style:style style:name="ce9" style:family="table-cell" style:parent-style-name="Default" style:data-style-name="N60"/>'
    '<style:style style:name="ce10" style:family="table-cell" style:parent-style-name="Default" style:data-style-name="N70"/>'
    '<style:style style:name="ce11" style:family="table-cell" style:parent-style-name="Default" style:data-style-name="N100"/>'
    '<style:style style:name="ce12" style:family="table-cell" style:parent-style-name="Default" style:data-style-name="N80"/>'
    '<style:style style:name="ce13" style:family="table-cell" style:parent-style-name="Default"><style:table-cell-properties style:text-align-source="value-type" style:repeat-content="false"/><style:paragraph-properties fo:text-align="center"/></style:style>'
)


def lo_content():
    r = []
    # Row 1: bold headers, rest of the row blank (columns-repeated).
    r.append('<table:table-row table:style-name="ro1">')
    for h in ["Item", "Amount", "Due", "Paid", "Ratio"]:
        r.append('<table:table-cell table:style-name="ce1" office:value-type="string" calcext:value-type="string"><text:p>%s</text:p></table:table-cell>' % h)
    r.append('<table:table-cell table:number-columns-repeated="1019"/></table:table-row>')
    # Row 2: span + currency + date + boolean.
    r.append('<table:table-row table:style-name="ro1">')
    r.append('<table:table-cell office:value-type="string" calcext:value-type="string"><text:p>Widget <text:span text:style-name="T1">blue</text:span></text:p></table:table-cell>')
    r.append('<table:table-cell table:style-name="ce2" office:value-type="currency" office:currency="USD" office:value="1234.5" calcext:value-type="currency"><text:p>$1,234.50</text:p></table:table-cell>')
    r.append('<table:table-cell table:style-name="ce3" office:value-type="date" office:date-value="2026-10-19" calcext:value-type="date"><text:p>10/19/2026</text:p></table:table-cell>')
    r.append('<table:table-cell office:value-type="boolean" office:boolean-value="true" calcext:value-type="boolean"><text:p>TRUE</text:p></table:table-cell>')
    r.append('<table:table-cell table:number-columns-repeated="1020"/></table:table-row>')
    # Row 3: text:s, negative currency, datetime, false.
    r.append('<table:table-row table:style-name="ro1">')
    r.append('<table:table-cell office:value-type="string" calcext:value-type="string"><text:p>Gadget <text:s text:c="2"/>two</text:p></table:table-cell>')
    r.append('<table:table-cell table:style-name="ce2" office:value-type="currency" office:currency="USD" office:value="-20" calcext:value-type="currency"><text:p>-$20.00</text:p></table:table-cell>')
    r.append('<table:table-cell table:style-name="ce3" office:value-type="date" office:date-value="2026-10-19T18:00:00" calcext:value-type="date"><text:p>10/19/2026</text:p></table:table-cell>')
    r.append('<table:table-cell office:value-type="boolean" office:boolean-value="false" calcext:value-type="boolean"><text:p>FALSE</text:p></table:table-cell>')
    r.append('<table:table-cell table:number-columns-repeated="1020"/></table:table-row>')
    # Row 4: formulas with cached values of every type.
    r.append('<table:table-row table:style-name="ro1">')
    r.append('<table:table-cell table:formula="of:=CONCATENATE(&quot;Tot&quot;;&quot;al&quot;)" office:value-type="string" office:string-value="Total" calcext:value-type="string"><text:p>Total</text:p></table:table-cell>')
    r.append('<table:table-cell table:style-name="ce2" table:formula="of:=SUM([.B2:.B3])" office:value-type="currency" office:currency="USD" office:value="1214.5" calcext:value-type="currency"><text:p>$1,214.50</text:p></table:table-cell>')
    r.append('<table:table-cell table:formula="of:=1/0" office:value-type="string" office:string-value="" calcext:value-type="error"><text:p>#DIV/0!</text:p></table:table-cell>')
    r.append('<table:table-cell table:formula="of:=[.D2]&lt;&gt;[.D3]" office:value-type="boolean" office:boolean-value="true" calcext:value-type="boolean"><text:p>TRUE</text:p></table:table-cell>')
    r.append('<table:table-cell table:style-name="ce4" table:formula="of:=[.B3]/[.B2]" office:value-type="percentage" office:value="-0.0162008910490077" calcext:value-type="percentage"><text:p>-1.62%</text:p></table:table-cell>')
    r.append('<table:table-cell table:number-columns-repeated="1019"/></table:table-row>')
    # Rows 5-7: a repeated non-empty row (two cells each).
    r.append('<table:table-row table:style-name="ro1" table:number-rows-repeated="3"><table:table-cell office:value-type="float" office:value="7" calcext:value-type="float" table:number-columns-repeated="2"><text:p>7</text:p></table:table-cell><table:table-cell table:number-columns-repeated="1022"/></table:table-row>')
    # Row 8: merge + covered cell, multi-paragraph text, tab/line-break/link, annotation.
    r.append('<table:table-row table:style-name="ro2">')
    r.append('<table:table-cell table:style-name="ce5" table:number-columns-spanned="2" table:number-rows-spanned="1" office:value-type="string" calcext:value-type="string"><text:p>Merged</text:p></table:table-cell><table:covered-table-cell/>')
    r.append('<table:table-cell office:value-type="string" calcext:value-type="string"><text:p>line1</text:p><text:p>line2</text:p></table:table-cell>')
    r.append('<table:table-cell office:value-type="string" calcext:value-type="string"><text:p>a<text:tab/>b<text:line-break/>c <text:a xlink:href="https://example.com/" xlink:type="simple">link</text:a></text:p></table:table-cell>')
    r.append('<table:table-cell office:value-type="string" calcext:value-type="string"><office:annotation draw:style-name="gr1" svg:width="2cm" svg:height="1cm" svg:x="1cm" svg:y="0cm"><dc:creator>Ann</dc:creator><dc:date>2026-10-19T09:00:00</dc:date><text:p>a note</text:p></office:annotation><text:p>with note</text:p></table:table-cell>')
    r.append('<table:table-cell table:number-columns-repeated="1019"/></table:table-row>')
    # Row 9: time, inherited common style, legacy oooc formula, escaped chars, styled empty cell.
    r.append('<table:table-row table:style-name="ro1">')
    r.append('<table:table-cell table:style-name="ce6" office:value-type="time" office:time-value="PT14H30M00S" calcext:value-type="time"><text:p>02:30 PM</text:p></table:table-cell>')
    r.append('<table:table-cell table:style-name="ce7" office:value-type="string" calcext:value-type="string"><text:p>Accent</text:p></table:table-cell>')
    r.append('<table:table-cell table:formula="oooc:=[.A5]*2" office:value-type="float" office:value="14" calcext:value-type="float"><text:p>14</text:p></table:table-cell>')
    r.append('<table:table-cell office:value-type="string" calcext:value-type="string"><text:p>&lt;tag&gt; &amp; &quot;q&quot; &#x263A;</text:p></table:table-cell>')
    r.append('<table:table-cell table:style-name="ce1"/>')
    r.append('<table:table-cell table:number-columns-repeated="1019"/></table:table-row>')
    # Row 10: more number formats.
    r.append('<table:table-row table:style-name="ro1">')
    r.append('<table:table-cell table:style-name="ce8" office:value-type="date" office:date-value="2000-02-29" calcext:value-type="date"><text:p>29 Feb 2000</text:p></table:table-cell>')
    r.append('<table:table-cell table:style-name="ce9" office:value-type="float" office:value="12345.678" calcext:value-type="float"><text:p>1.23E+04</text:p></table:table-cell>')
    r.append('<table:table-cell table:style-name="ce10" office:value-type="float" office:value="1.5" calcext:value-type="float"><text:p>1 1/2</text:p></table:table-cell>')
    r.append('<table:table-cell table:style-name="ce11" office:value-type="string" calcext:value-type="string"><text:p>00123</text:p></table:table-cell>')
    r.append('<table:table-cell table:style-name="ce12" office:value-type="float" office:value="3" calcext:value-type="float"><text:p>Qty 3</text:p></table:table-cell>')
    r.append('<table:table-cell table:style-name="ce13" office:value-type="float" office:value="1" calcext:value-type="float"><text:p>1</text:p></table:table-cell>')
    r.append('<table:table-cell table:number-columns-repeated="1018"/></table:table-row>')
    # The huge empty tail LibreOffice always writes.
    r.append('<table:table-row table:style-name="ro1" table:number-rows-repeated="1048565"><table:table-cell table:number-columns-repeated="1024"/></table:table-row>')
    r.append('<table:table-row table:style-name="ro1"><table:table-cell table:number-columns-repeated="1024"/></table:table-row>')
    orders = (
        '<table:table table:name="Orders" table:style-name="ta1">'
        '<table:table-column table:style-name="co1" table:default-cell-style-name="Default"/>'
        '<table:table-column table:style-name="co2" table:number-columns-repeated="2" table:default-cell-style-name="Default"/>'
        '<table:table-column table:style-name="co3" table:default-cell-style-name="ce1"/>'
        '<table:table-column table:style-name="co2" table:number-columns-repeated="1020" table:default-cell-style-name="Default"/>'
        + "".join(r)
        + "</table:table>"
    )
    summary = (
        '<table:table table:name="Summary &amp; Notes" table:style-name="ta2">'
        '<table:table-column table:style-name="co2" table:number-columns-repeated="1024" table:default-cell-style-name="Default"/>'
        '<table:table-header-rows><table:table-row table:style-name="ro1">'
        '<table:table-cell table:formula="of:=SUM([$Orders.B2:.B3])" office:value-type="float" office:value="1214.5" calcext:value-type="float"><text:p>1214.5</text:p></table:table-cell>'
        '<table:table-cell table:number-columns-repeated="1023"/></table:table-row></table:table-header-rows>'
        '<table:table-row table:style-name="ro1" table:number-rows-repeated="1048575"><table:table-cell table:number-columns-repeated="1024"/></table:table-row>'
        "</table:table>"
        '<table:named-expressions><table:named-range table:name="Total" table:base-cell-address="$Orders.$A$1" table:cell-range-address="$Orders.$B$2:.$B$3"/></table:named-expressions>'
    )
    calc = '<table:calculation-settings table:automatic-find-labels="false" table:use-regular-expressions="false" table:use-wildcards="true" table:case-sensitive="false"/>'
    return content_doc(orders + summary, LO_AUTO, LO_FONTS, "1.3", calc)


LO_STYLES = (
    DECL
    + '<office:document-styles%s office:version="1.3">' % NS
    + "<office:font-face-decls>%s</office:font-face-decls>" % LO_FONTS
    + "<office:styles>"
    + '<style:default-style style:family="table-cell"><style:paragraph-properties style:tab-stop-distance="1.25cm"/><style:text-properties style:font-name="Liberation Sans" fo:font-size="10pt" fo:language="en" fo:country="US"/></style:default-style>'
    + '<number:number-style style:name="N0"><number:number number:min-integer-digits="1"/></number:number-style>'
    + '<style:style style:name="Default" style:family="table-cell"><style:text-properties fo:font-size="10pt"/></style:style>'
    + '<style:style style:name="Heading" style:family="table-cell" style:parent-style-name="Default"><style:text-properties fo:color="#000000" fo:font-size="24pt" fo:font-style="normal" fo:font-weight="bold"/></style:style>'
    + '<style:style style:name="Accent" style:family="table-cell" style:parent-style-name="Default"><style:text-properties fo:color="#000000" fo:font-weight="bold"/></style:style>'
    + "</office:styles>"
    + '<office:automatic-styles><style:page-layout style:name="Mpm1"><style:page-layout-properties style:writing-mode="lr-tb"/></style:page-layout></office:automatic-styles>'
    + '<office:master-styles><style:master-page style:name="Default" style:page-layout-name="Mpm1"><style:header><text:p><text:sheet-name>???</text:sheet-name></text:p></style:header></style:master-page></office:master-styles>'
    + "</office:document-styles>"
)


def meta(generator, created="2026-10-18T10:00:00.123456789", modified="2026-10-19T08:30:00.5", title=None):
    t = "<dc:title>%s</dc:title>" % title if title else ""
    return (
        DECL
        + '<office:document-meta%s office:version="1.3"><office:meta>' % NS
        + "<meta:creation-date>%s</meta:creation-date><dc:date>%s</dc:date>" % (created, modified)
        + "<meta:editing-duration>PT5M</meta:editing-duration><meta:editing-cycles>3</meta:editing-cycles>"
        + "<meta:generator>%s</meta:generator>%s" % (generator, t)
        + '<meta:document-statistic meta:table-count="2" meta:cell-count="40" meta:object-count="0"/>'
        + "</office:meta></office:document-meta>"
    )


def settings(active, tables, cursor=(0, 0)):
    def item(n, t, v):
        return '<config:config-item config:name="%s" config:type="%s">%s</config:config-item>' % (n, t, v)

    entries = []
    for name, (hmode, hpos, vmode, vpos) in tables:
        entries.append(
            '<config:config-item-map-entry config:name="%s">' % name
            + item("CursorPositionX", "int", cursor[0])
            + item("CursorPositionY", "int", cursor[1])
            + item("HorizontalSplitMode", "short", hmode)
            + item("VerticalSplitMode", "short", vmode)
            + item("HorizontalSplitPosition", "int", hpos)
            + item("VerticalSplitPosition", "int", vpos)
            + item("ActiveSplitRange", "short", 2)
            + item("PositionLeft", "int", 0)
            + item("PositionRight", "int", hpos)
            + item("PositionTop", "int", 0)
            + item("PositionBottom", "int", vpos)
            + item("ZoomType", "short", 0)
            + item("ZoomValue", "int", 100)
            + "</config:config-item-map-entry>"
        )
    return (
        DECL
        + '<office:document-settings%s office:version="1.3"><office:settings>' % NS
        + '<config:config-item-set config:name="ooo:view-settings">'
        + item("VisibleAreaTop", "int", 0)
        + '<config:config-item-map-indexed config:name="Views"><config:config-item-map-entry>'
        + item("ViewId", "string", "view1")
        + '<config:config-item-map-named config:name="Tables">'
        + "".join(entries)
        + "</config:config-item-map-named>"
        + item("ActiveTable", "string", active)
        + item("HorizontalScrollbarWidth", "int", 1309)
        + item("ShowGrid", "boolean", "true")
        + "</config:config-item-map-entry></config:config-item-map-indexed></config:config-item-set>"
        + '<config:config-item-set config:name="ooo:configuration-settings">'
        + item("ActiveTable", "string", "NotAView")
        + item("HasColumnRowHeaders", "boolean", "true")
        + "</config:config-item-set>"
        + "</office:settings></office:document-settings>"
    )


def gen_libreoffice():
    write_zip(
        os.path.join(ODS_DIR, "libreoffice-style.ods"),
        [
            ("META-INF/manifest.xml", manifest("1.3", ["Thumbnails/thumbnail.png", "Configurations2/"])),
            ("Configurations2/", b""),
            ("Thumbnails/thumbnail.png", b"\x89PNG\r\n\x1a\n" + b"\0" * 32),
            ("content.xml", lo_content()),
            ("styles.xml", LO_STYLES),
            ("meta.xml", meta("LibreOffice/7.6.4.1$Linux_X86_64 LibreOffice_project/e19e193f88cd6c0525a17fb7a176ed8e6a3e2aa1")),
            ("settings.xml", settings("Summary &amp; Notes", [("Orders", (0, 0, 2, 1)), ("Summary &amp; Notes", (0, 0, 0, 0))])),
        ],
    )


# ---------------------------------------------------------------------------
# Apache OpenOffice 4.1 shaped document
# ---------------------------------------------------------------------------


def gen_aoo():
    auto = (
        '<style:style style:name="co1" style:family="table-column"><style:table-column-properties fo:break-before="auto" style:column-width="0.8925in"/></style:style>'
        '<style:style style:name="ro1" style:family="table-row"><style:table-row-properties style:row-height="0.1783in" fo:break-before="auto" style:use-optimal-row-height="true"/></style:style>'
        '<style:style style:name="ta1" style:family="table" style:master-page-name="Default"><style:table-properties table:display="true" style:writing-mode="lr-tb"/></style:style>'
        '<style:style style:name="ta2" style:family="table" style:master-page-name="Default"><style:table-properties table:display="false" style:writing-mode="lr-tb"/></style:style>'
        '<number:number-style style:name="N3"><number:number number:decimal-places="0" number:min-integer-digits="1" number:grouping="true"/></number:number-style>'
        '<number:date-style style:name="N5" number:automatic-order="true"><number:year number:style="long"/><number:text>-</number:text><number:month number:style="long"/><number:text>-</number:text><number:day number:style="long"/></number:date-style>'
        '<number:time-style style:name="N6"><number:hours number:style="long"/><number:text>:</number:text><number:minutes number:style="long"/><number:text>:</number:text><number:seconds number:style="long"/></number:time-style>'
        '<number:percentage-style style:name="N7"><number:number number:decimal-places="0" number:min-integer-digits="1"/><number:text>%</number:text></number:percentage-style>'
        '<style:style style:name="ce1" style:family="table-cell" style:parent-style-name="Default" style:data-style-name="N3"/>'
        '<style:style style:name="ce2" style:family="table-cell" style:parent-style-name="Default" style:data-style-name="N5"/>'
        '<style:style style:name="ce3" style:family="table-cell" style:parent-style-name="Default" style:data-style-name="N6"/>'
        '<style:style style:name="ce4" style:family="table-cell" style:parent-style-name="Default" style:data-style-name="N7"/>'
        '<style:style style:name="ce5" style:family="table-cell" style:parent-style-name="Default"><style:text-properties fo:font-weight="bold" style:font-weight-asian="bold" style:font-weight-complex="bold"/></style:style>'
    )
    rows = (
        '<table:table-row table:style-name="ro1"><table:table-cell table:style-name="ce5" office:value-type="string"><text:p>Qty</text:p></table:table-cell><table:table-cell table:style-name="ce5" office:value-type="string"><text:p>When</text:p></table:table-cell><table:table-cell table:style-name="ce5" office:value-type="string"><text:p>At</text:p></table:table-cell><table:table-cell table:number-columns-repeated="1021"/></table:table-row>'
        '<table:table-row table:style-name="ro1"><table:table-cell table:style-name="ce1" office:value-type="float" office:value="1234567"><text:p>1,234,567</text:p></table:table-cell><table:table-cell table:style-name="ce2" office:value-type="date" office:date-value="1999-12-31"><text:p>1999-12-31</text:p></table:table-cell><table:table-cell table:style-name="ce3" office:value-type="time" office:time-value="PT23H59M59S"><text:p>23:59:59</text:p></table:table-cell><table:table-cell table:number-columns-repeated="1021"/></table:table-row>'
        '<table:table-row table:style-name="ro1"><table:table-cell table:formula="of:=SUM([.A2];1)" office:value-type="float" office:value="1234568"><text:p>1234568</text:p></table:table-cell><table:table-cell table:formula="of:=1/0" office:value-type="float" office:value="0"><text:p>#DIV/0!</text:p></table:table-cell><table:table-cell table:formula="of:=NA()" office:value-type="string" office:string-value=""><text:p>#N/A</text:p></table:table-cell><table:table-cell table:style-name="ce4" table:formula="of:=0.25" office:value-type="percentage" office:value="0.25"><text:p>25%</text:p></table:table-cell><table:table-cell table:number-columns-repeated="1020"/></table:table-row>'
        '<table:table-row table:style-name="ro1" table:number-rows-repeated="65533"><table:table-cell table:number-columns-repeated="1024"/></table:table-row>'
    )
    body = (
        '<table:table table:name="Data" table:style-name="ta1" table:print="false">'
        '<table:table-column table:style-name="co1" table:number-columns-repeated="1024" table:default-cell-style-name="Default"/>'
        + rows
        + "</table:table>"
        '<table:table table:name="Hidden" table:style-name="ta2" table:print="false">'
        '<table:table-column table:style-name="co1" table:default-cell-style-name="Default"/>'
        '<table:table-row table:style-name="ro1"><table:table-cell office:value-type="string"><text:p>secret</text:p></table:table-cell></table:table-row>'
        "</table:table>"
    )
    calc = '<table:calculation-settings table:case-sensitive="false" table:search-criteria-must-apply-to-whole-cell="false" table:use-regular-expressions="true"><table:null-date table:date-value="1899-12-30"/><table:iteration table:status="enable" table:steps="50" table:minimum-difference="0.0001"/></table:calculation-settings>'
    styles = (
        DECL
        + '<office:document-styles%s office:version="1.2">' % NS
        + '<office:styles><style:default-style style:family="table-cell"><style:text-properties style:font-name="Arial" fo:font-size="10pt"/></style:default-style><style:style style:name="Default" style:family="table-cell"/></office:styles>'
        + "</office:document-styles>"
    )
    write_zip(
        os.path.join(ODS_DIR, "aoo-style.ods"),
        [
            ("META-INF/manifest.xml", manifest("1.2")),
            ("content.xml", content_doc(body, auto, "", "1.2", calc)),
            ("styles.xml", styles),
            ("meta.xml", meta("OpenOffice/4.1.15$Win32 OpenOffice.org_project/4115m1$Build-9813", title="AOO sample")),
            ("settings.xml", settings("Data", [("Data", (2, 1, 2, 1)), ("Hidden", (0, 0, 0, 0))])),
        ],
    )


def gen_legacy_and_minimal():
    # OpenOffice.org 2.x style: oooc:= formulas, office:version 1.1, 1904 null date.
    rows = (
        '<table:table-row><table:table-cell office:value-type="float" office:value="2"><text:p>2</text:p></table:table-cell>'
        '<table:table-cell office:value-type="float" office:value="3"><text:p>3</text:p></table:table-cell>'
        '<table:table-cell table:formula="oooc:=SUM([.A1:.B1])" office:value-type="float" office:value="5"><text:p>5</text:p></table:table-cell></table:table-row>'
        '<table:table-row><table:table-cell office:value-type="date" office:date-value="1904-01-02"><text:p>01/02/04</text:p></table:table-cell></table:table-row>'
    )
    calc = '<table:calculation-settings><table:null-date table:date-value="1904-01-01"/></table:calculation-settings>'
    body = simple_table("Legacy", rows)
    write_zip(
        os.path.join(ODS_DIR, "ooo2-legacy.ods"),
        [
            ("META-INF/manifest.xml", manifest("1.1")),
            ("content.xml", content_doc(body, BASE_AUTO, "", "1.1", calc)),
        ],
    )
    # Minimal: mimetype last and deflated, only content.xml, a cell with no value-type.
    body = (
        '<table:table table:name="Only">'
        '<table:table-row><table:table-cell><text:p>plain text</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="1e300"/></table:table-row>'
        "</table:table>"
    )
    write_zip(
        os.path.join(ODS_DIR, "minimal-no-manifest.ods"),
        [("content.xml", content_doc(body))],
        mimetype_first=False,
        mimetype_method=zipfile.ZIP_DEFLATED,
    )


# ---------------------------------------------------------------------------
# Malicious corpus
# ---------------------------------------------------------------------------

CONTENT_HEAD = DECL + '<office:document-content%s office:version="1.2"><office:body><office:spreadsheet>' % NS
CONTENT_TAIL = "</office:spreadsheet></office:body></office:document-content>"


def gen_zip_bomb():
    path = os.path.join(BAD_DIR, "zip-bomb.ods")
    with zipfile.ZipFile(path, "w") as z:
        z.writestr(info("mimetype", zipfile.ZIP_STORED), MIMETYPE)
        zi = info("content.xml")
        zi.compress_level = 9
        chunk = b"\0" * (16 * 1024 * 1024)
        with z.open(zi, "w") as f:
            for _ in range(64):  # 1 GiB of zeros
                f.write(chunk)


def gen_doctype():
    xxe = (
        DECL
        + '<!DOCTYPE office:document-content [<!ENTITY xxe SYSTEM "file:///etc/passwd">]>'
        + '<office:document-content%s office:version="1.2"><office:body><office:spreadsheet>' % NS
        + '<table:table table:name="S"><table:table-row><table:table-cell office:value-type="string"><text:p>&xxe;</text:p></table:table-cell></table:table-row></table:table>'
        + CONTENT_TAIL
    )
    write_zip(os.path.join(BAD_DIR, "doctype-xxe.ods"), [("content.xml", xxe)])
    lol = ['<!ENTITY lol "lol">']
    for i in range(1, 10):
        prev = "lol" if i == 1 else "lol%d" % (i - 1)
        lol.append('<!ENTITY lol%d "%s">' % (i, ("&%s;" % prev) * 10))
    laughs = (
        DECL
        + "<!DOCTYPE lolz [%s]>" % "".join(lol)
        + '<office:document-content%s office:version="1.2"><office:body><office:spreadsheet>' % NS
        + '<table:table table:name="S"><table:table-row><table:table-cell office:value-type="string"><text:p>&lol9;</text:p></table:table-cell></table:table-row></table:table>'
        + CONTENT_TAIL
    )
    write_zip(os.path.join(BAD_DIR, "billion-laughs.ods"), [("content.xml", laughs)])
    good = content_doc(simple_table("S", "<table:table-row>%s</table:table-row>" % str_cell("x")), BASE_AUTO)
    styles = DECL + '<!DOCTYPE x [<!ENTITY e SYSTEM "http://127.0.0.1:9/x">]><office:document-styles%s>&e;</office:document-styles>' % NS
    write_zip(os.path.join(BAD_DIR, "doctype-in-styles.ods"), [("content.xml", good), ("styles.xml", styles)])


def gen_path_traversal():
    good = content_doc(simple_table("Good", "<table:table-row>%s</table:table-row>" % str_cell("safe")), BASE_AUTO)
    evil = content_doc(simple_table("Evil", "<table:table-row>%s</table:table-row>" % str_cell("pwned")), BASE_AUTO)
    write_zip(
        os.path.join(BAD_DIR, "path-traversal.ods"),
        [
            ("../content.xml", evil),
            ("../../../../tmp/tally-evil.txt", b"evil"),
            ("/etc/tally-evil", b"evil"),
            ("..\\..\\tally-evil.txt", b"evil"),
            ("C:/tally-evil.txt", b"evil"),
            ("a/../../styles.xml", b"<!DOCTYPE x []><x/>"),
            ("content.xml", good),
        ],
    )


def gen_repeat_bomb():
    row = (
        '<table:table-row table:number-rows-repeated="1048576">'
        '<table:table-cell table:number-columns-repeated="16384" office:value-type="float" office:value="1"><text:p>1</text:p></table:table-cell>'
        "</table:table-row>"
    )
    write_zip(os.path.join(BAD_DIR, "repeat-bomb.ods"), [("content.xml", content_doc(simple_table("Bomb", row), BASE_AUTO))])
    # Whole-sheet spaces bomb: one cell whose text:s claims 4 billion spaces.
    sp = '<table:table-row><table:table-cell office:value-type="string"><text:p>a<text:s text:c="4000000000"/>b</text:p></table:table-cell></table:table-row>'
    write_zip(os.path.join(BAD_DIR, "space-bomb.ods"), [("content.xml", content_doc(simple_table("Spaces", sp), BASE_AUTO))])


def gen_deep_nesting():
    n = 100_000
    inner = "<text:span>" * n + "deep" + "</text:span>" * n
    row = '<table:table-row><table:table-cell office:value-type="string"><text:p>%s</text:p></table:table-cell></table:table-row>' % inner
    write_zip(os.path.join(BAD_DIR, "deep-nesting.ods"), [("content.xml", CONTENT_HEAD + simple_table("Deep", row) + CONTENT_TAIL)])
    attrs = " ".join('a%d="x"' % i for i in range(300))
    row = '<table:table-row><table:table-cell %s office:value-type="string"><text:p>x</text:p></table:table-cell></table:table-row>' % attrs
    write_zip(os.path.join(BAD_DIR, "many-attributes.ods"), [("content.xml", CONTENT_HEAD + simple_table("Attrs", row) + CONTENT_TAIL)])


def patch_sizes(path, entry, fake_size):
    """Rewrites the uncompressed-size fields of `entry` in the local header and
    the central directory so the zip lies about how big the entry inflates."""
    data = bytearray(open(path, "rb").read())
    name = entry.encode()
    # Local file header: sig PK\3\4, uncompressed size at +22, name at +30.
    i = 0
    while True:
        i = data.find(b"PK\x03\x04", i)
        if i < 0:
            break
        nlen = struct.unpack_from("<H", data, i + 26)[0]
        if data[i + 30 : i + 30 + nlen] == name:
            struct.pack_into("<I", data, i + 22, fake_size)
        i += 4
    # Central directory header: sig PK\1\2, uncompressed size at +24, name at +46.
    i = 0
    while True:
        i = data.find(b"PK\x01\x02", i)
        if i < 0:
            break
        nlen = struct.unpack_from("<H", data, i + 28)[0]
        if data[i + 46 : i + 46 + nlen] == name:
            struct.pack_into("<I", data, i + 24, fake_size)
        i += 4
    open(path, "wb").write(bytes(data))


def gen_lying_size():
    big = "x" * (8 * 1024 * 1024)
    row = '<table:table-row><table:table-cell office:value-type="string"><text:p>%s</text:p></table:table-cell></table:table-row>' % big
    path = os.path.join(BAD_DIR, "lying-size.ods")
    write_zip(path, [("content.xml", content_doc(simple_table("Liar", row), BASE_AUTO))])
    patch_sizes(path, "content.xml", 100)


def gen_many_entries():
    path = os.path.join(BAD_DIR, "many-entries.ods")
    good = content_doc(simple_table("S", "<table:table-row>%s</table:table-row>" % str_cell("x")), BASE_AUTO)
    with zipfile.ZipFile(path, "w") as z:
        z.writestr(info("mimetype", zipfile.ZIP_STORED), MIMETYPE)
        z.writestr(info("content.xml"), good)
        for i in range(20_000):
            z.writestr(info("e/%05d" % i, zipfile.ZIP_STORED), b"")


def gen_not_ods():
    open(os.path.join(BAD_DIR, "not-a-zip.ods"), "wb").write(b"This is plain text pretending to be an .ods file.\n")
    write_zip(os.path.join(BAD_DIR, "no-content.ods"), [("META-INF/manifest.xml", manifest("1.2")), ("styles.xml", LO_STYLES)])
    text_doc = (
        DECL + '<office:document-content%s office:version="1.2"><office:body><office:text><text:p>hello</text:p></office:text></office:body></office:document-content>' % NS
    )
    with zipfile.ZipFile(os.path.join(BAD_DIR, "text-document.odt"), "w") as z:
        z.writestr(info("mimetype", zipfile.ZIP_STORED), b"application/vnd.oasis.opendocument.text")
        z.writestr(info("content.xml"), text_doc)


# ---------------------------------------------------------------------------
# CSV fixtures
# ---------------------------------------------------------------------------


def gen_csv():
    def w(name, data):
        open(os.path.join(CSV_DIR, name), "wb").write(data)

    w("utf8-bom.csv", b"\xef\xbb\xbfname,city\r\nJos\xc3\xa9,S\xc3\xa3o Paulo\r\n")
    w("utf16le-bom.csv", b"\xff\xfe" + "name,city\r\nJosé,Zürich\r\n".encode("utf-16-le"))
    w("utf16be-bom.csv", b"\xfe\xff" + "name;city\nJosé;Zürich\n".encode("utf-16-be"))
    w("latin1.csv", "name,price\ncaf\u00e9,3\n".encode("latin-1"))
    w("windows1252.csv", b"item,price\nbook,\x8020\n\x93quoted\x94,1\n")
    w("semicolon.csv", "a;b;c\n1,5;2,5;3\n4;5;6\n".encode())
    w("tabs.tsv", b"a\tb\tc\n1\t2\t3\n")
    w("quoted-newlines.csv", b'id,note\r\n1,"line one\r\nline two"\r\n2,"say ""hi"""\r\n3,\r\n')
    w("ragged.csv", b"a,b,c\n1\n1,2,3,4,5\n\n,\n")
    w("injection.csv", b'name,formula\nx,=1+2\ny,"=HYPERLINK(""http://evil"")"\nz,+cmd\n')
    w("empty.csv", b"")


# ---------------------------------------------------------------------------
# Charts (LibreOffice 7.x shaped embedded chart objects; wave 5, W5-1)
# ---------------------------------------------------------------------------

CHART_NS = (
    ' xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"'
    ' xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"'
    ' xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"'
    ' xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"'
    ' xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"'
    ' xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"'
    ' xmlns:xlink="http://www.w3.org/1999/xlink"'
    ' xmlns:number="urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0"'
    ' xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0"'
    ' xmlns:chart="urn:oasis:names:tc:opendocument:xmlns:chart:1.0"'
    ' xmlns:ooo="http://openoffice.org/2004/office"'
    ' xmlns:loext="urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0"'
)

CHART_STYLES_XML = (
    DECL
    + '<office:document-styles%s office:version="1.3"><office:styles/><office:automatic-styles/><office:master-styles/></office:document-styles>' % CHART_NS
)

PNG_1x1 = (
    b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89"
    b"\x00\x00\x00\rIDATx\x9cc\xf8\xcf\xc0\xf0\x1f\x00\x05\x00\x02\x00\xde\xf2\x8a\xbb\x00\x00\x00\x00IEND\xaeB`\x82"
)


def chart_doc(auto_styles, body):
    return (
        DECL
        + '<office:document-content%s office:version="1.3">' % CHART_NS
        + "<office:automatic-styles>%s</office:automatic-styles>" % auto_styles
        + "<office:body><office:chart>%s</office:chart></office:body></office:document-content>" % body
    )


def lo_column_chart():
    """A clustered column chart of Data.A1:C6 with a title, legend, Y grid,
    data labels on the second series and a data style on the Y axis (the
    shape LibreOffice 7.6 writes)."""
    auto = (
        '<number:number-style style:name="N0"><number:number number:min-integer-digits="1"/></number:number-style>'
        '<style:style style:name="ch1" style:family="chart"><style:graphic-properties draw:stroke="none" draw:fill="none"/></style:style>'
        '<style:style style:name="ch2" style:family="chart"><style:chart-properties chart:auto-position="true"/><style:text-properties fo:font-size="13pt" style:font-size-asian="13pt" style:font-size-complex="13pt"/></style:style>'
        '<style:style style:name="ch3" style:family="chart"><style:chart-properties chart:auto-position="true"/><style:text-properties fo:font-size="10pt"/></style:style>'
        '<style:style style:name="ch4" style:family="chart"><style:chart-properties chart:stacked="false" chart:percentage="false" chart:three-dimensional="false" chart:vertical="false" chart:right-angled-axes="true" chart:include-hidden-cells="false" chart:auto-position="true" chart:auto-size="true" chart:treat-empty-cells="leave-gap"/></style:style>'
        '<style:style style:name="ch5" style:family="chart" style:data-style-name="N0"><style:chart-properties chart:display-label="true" chart:logarithmic="false" chart:reverse-direction="false" chart:axis-position="start"/><style:graphic-properties svg:stroke-color="#b3b3b3"/><style:text-properties fo:font-size="10pt"/></style:style>'
        '<style:style style:name="ch6" style:family="chart"><style:chart-properties chart:display-label="true" chart:logarithmic="false" chart:minimum="0" chart:maximum="120" chart:interval-major="20" chart:reverse-direction="false" chart:axis-position="0"/><style:graphic-properties svg:stroke-color="#b3b3b3"/><style:text-properties fo:font-size="10pt"/></style:style>'
        '<style:style style:name="ch7" style:family="chart"><style:graphic-properties svg:stroke-color="#b3b3b3"/></style:style>'
        '<style:style style:name="ch8" style:family="chart"><style:chart-properties chart:symbol-type="none" chart:link-data-style-to-source="true"/><style:graphic-properties draw:fill-color="#004586" svg:stroke-color="#004586"/></style:style>'
        '<style:style style:name="ch9" style:family="chart"><style:chart-properties chart:data-label-number="value" chart:data-label-text="false" chart:data-label-symbol="false" chart:label-position="outside" chart:symbol-type="none" chart:link-data-style-to-source="true"/><style:graphic-properties draw:fill-color="#ff420e" svg:stroke-color="#ff420e"/></style:style>'
        '<style:style style:name="ch10" style:family="chart"><style:graphic-properties draw:fill="solid" draw:fill-color="#e6e6e6" draw:stroke="none"/></style:style>'
    )
    body = (
        '<chart:chart svg:width="16cm" svg:height="9cm" xlink:href=".." xlink:type="simple" chart:class="chart:bar" chart:style-name="ch1">'
        '<chart:title svg:x="6cm" svg:y="0.2cm" chart:style-name="ch2"><text:p>Sales <text:span text:style-name="T1">2026</text:span></text:p></chart:title>'
        '<chart:legend chart:legend-position="end" svg:x="13cm" svg:y="4cm" style:legend-expansion="high" chart:style-name="ch3"/>'
        '<chart:plot-area chart:style-name="ch4" table:cell-range-address="Data.A1:Data.C6" chart:data-source-has-labels="both" svg:x="1cm" svg:y="1cm" svg:width="11cm" svg:height="7cm">'
        '<chart:axis chart:dimension="x" chart:name="primary-x" chart:style-name="ch5"><chart:title chart:style-name="ch3"><text:p>Month</text:p></chart:title><chart:categories table:cell-range-address="Data.A2:Data.A6"/></chart:axis>'
        '<chart:axis chart:dimension="y" chart:name="primary-y" chart:style-name="ch6"><chart:title chart:style-name="ch3"><text:p>k€</text:p></chart:title><chart:grid chart:style-name="ch7" chart:class="major"/></chart:axis>'
        '<chart:series chart:style-name="ch8" chart:values-cell-range-address="Data.B2:Data.B6" chart:label-cell-address="Data.B1" chart:class="chart:bar"><chart:data-point chart:repeated="5"/></chart:series>'
        '<chart:series chart:style-name="ch9" chart:values-cell-range-address="Data.C2:Data.C6" chart:label-cell-address="Data.C1" chart:class="chart:bar"><chart:data-point chart:repeated="5"/></chart:series>'
        '<chart:wall chart:style-name="ch10"/><chart:floor chart:style-name="ch10"/></chart:plot-area>'
        '<table:table table:name="local-table"><table:table-header-columns><table:table-column/></table:table-header-columns><table:table-columns><table:table-column table:number-columns-repeated="2"/></table:table-columns>'
        '<table:table-header-rows><table:table-row><table:table-cell><text:p/></table:table-cell><table:table-cell office:value-type="string"><text:p>Sales</text:p></table:table-cell><table:table-cell office:value-type="string"><text:p>Costs</text:p></table:table-cell></table:table-row></table:table-header-rows>'
        '<table:table-rows>'
        + "".join(
            '<table:table-row><table:table-cell office:value-type="string"><text:p>%s</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="%s"><text:p>%s</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="%s"><text:p>%s</text:p></table:table-cell></table:table-row>'
            % (m, a, a, b, b)
            for m, a, b in [("Jan", 10, 4), ("Feb", 20, 8), ("Mar", 35, 12), ("Apr", 50, 20), ("May", 100, 40)]
        )
        + "</table:table-rows></table:table></chart:chart>"
    )
    return chart_doc(auto, body)


def lo_pie_chart():
    """A 3D exploded pie of the Sales column (page anchored in table:shapes)."""
    auto = (
        '<style:style style:name="ch1" style:family="chart"><style:graphic-properties draw:stroke="none" draw:fill="solid" draw:fill-color="#ffffcc"/></style:style>'
        '<style:style style:name="ch2" style:family="chart"><style:chart-properties chart:three-dimensional="true" chart:deep="false" chart:sort-by-x-values="false" chart:right-angled-axes="true"/></style:style>'
        '<style:style style:name="ch3" style:family="chart"><style:chart-properties chart:pie-offset="20" chart:data-label-number="percentage" chart:data-label-text="true" chart:label-position="outside"/></style:style>'
    )
    body = (
        '<chart:chart svg:width="8cm" svg:height="7cm" xlink:href=".." xlink:type="simple" chart:class="chart:circle" chart:style-name="ch1">'
        '<chart:legend chart:legend-position="bottom"/>'
        '<chart:plot-area chart:style-name="ch2" table:cell-range-address="Data.A1:Data.B6" chart:data-source-has-labels="both">'
        '<chart:axis chart:dimension="x" chart:name="primary-x"><chart:categories table:cell-range-address="Data.A2:Data.A6"/></chart:axis>'
        '<chart:axis chart:dimension="y" chart:name="primary-y"/>'
        '<chart:series chart:style-name="ch3" chart:values-cell-range-address="Data.B2:Data.B6" chart:label-cell-address="Data.B1" chart:class="chart:circle"><chart:data-point chart:repeated="5"/></chart:series>'
        '<chart:wall/><chart:floor/></chart:plot-area></chart:chart>'
    )
    return chart_doc(auto, body)


def lo_line_chart():
    """A line chart with a smooth second series on the secondary Y axis
    (logarithmic), a linear regression with equation and R², standard
    deviation error bars and a mean value line."""
    auto = (
        '<style:style style:name="ch1" style:family="chart"><style:graphic-properties draw:stroke="none" draw:fill="none"/></style:style>'
        '<style:style style:name="ch2" style:family="chart"><style:chart-properties chart:stacked="false" chart:percentage="false" chart:three-dimensional="false" chart:symbol-type="automatic" chart:interpolation="cubic-spline" chart:lines="true"/></style:style>'
        '<style:style style:name="ch3" style:family="chart"><style:chart-properties chart:display-label="true" chart:logarithmic="false"/></style:style>'
        '<style:style style:name="ch4" style:family="chart"><style:chart-properties chart:display-label="true" chart:logarithmic="true" chart:minimum="1" chart:maximum="1000"/></style:style>'
        '<style:style style:name="ch5" style:family="chart"><style:chart-properties chart:symbol-type="named-symbol" chart:symbol-name="diamond" chart:symbol-width="0.25cm" chart:symbol-height="0.25cm"/><style:graphic-properties svg:stroke-color="#579d1c"/></style:style>'
        '<style:style style:name="ch6" style:family="chart"><style:chart-properties chart:regression-type="linear"/><style:graphic-properties svg:stroke-color="#000000"/></style:style>'
        '<style:style style:name="ch7" style:family="chart"><style:chart-properties chart:error-category="standard-deviation" chart:error-upper-indicator="true" chart:error-lower-indicator="true"/></style:style>'
        '<style:style style:name="ch8" style:family="chart"><style:chart-properties chart:symbol-type="none"/><style:graphic-properties svg:stroke-color="#7e0021"/></style:style>'
        '<style:style style:name="ch9" style:family="chart"><style:graphic-properties svg:stroke-color="#b3b3b3"/></style:style>'
    )
    body = (
        '<chart:chart svg:width="12cm" svg:height="8cm" xlink:href=".." xlink:type="simple" chart:class="chart:line" chart:style-name="ch1">'
        '<chart:title><text:p>Trend</text:p></chart:title><chart:subtitle><text:p>with regression</text:p></chart:subtitle>'
        '<chart:legend chart:legend-position="top"/>'
        '<chart:plot-area chart:style-name="ch2" table:cell-range-address="Data.A1:Data.C6" chart:data-source-has-labels="both">'
        '<chart:axis chart:dimension="x" chart:name="primary-x" chart:style-name="ch3"><chart:categories table:cell-range-address="Data.A2:Data.A6"/><chart:grid chart:class="minor"/></chart:axis>'
        '<chart:axis chart:dimension="y" chart:name="primary-y" chart:style-name="ch3"><chart:grid chart:style-name="ch9" chart:class="major"/></chart:axis>'
        '<chart:axis chart:dimension="y" chart:name="secondary-y" chart:style-name="ch4"/>'
        '<chart:series chart:style-name="ch5" chart:values-cell-range-address="Data.B2:Data.B6" chart:label-cell-address="Data.B1" chart:class="chart:line">'
        '<chart:regression-curve chart:style-name="ch6"><chart:equation chart:display-equation="true" chart:display-r-square="true"/></chart:regression-curve>'
        '<chart:error-indicator chart:style-name="ch7" chart:dimension="y"/><chart:mean-value chart:style-name="ch9"/><chart:data-point chart:repeated="5"/></chart:series>'
        '<chart:series chart:style-name="ch8" chart:values-cell-range-address="Data.C2:Data.C6" chart:label-cell-address="Data.C1" chart:class="chart:line" chart:attached-axis="secondary-y"><chart:data-point chart:repeated="5"/></chart:series>'
        '<chart:wall/><chart:floor/></chart:plot-area></chart:chart>'
    )
    return chart_doc(auto, body)


def gen_charts():
    d = os.path.join(ODS_DIR, "charts")
    os.makedirs(d, exist_ok=True)
    auto = (
        '<style:style style:name="co1" style:family="table-column"><style:table-column-properties style:column-width="2.258cm"/></style:style>'
        '<style:style style:name="ro1" style:family="table-row"><style:table-row-properties style:row-height="0.452cm" style:use-optimal-row-height="true"/></style:style>'
        '<style:style style:name="ta1" style:family="table" style:master-page-name="Default"><style:table-properties table:display="true" style:writing-mode="lr-tb"/></style:style>'
        '<style:style style:name="gr1" style:family="graphic"><style:graphic-properties draw:stroke="none" draw:fill="none"/></style:style>'
    )
    data = [("Jan", 10, 4), ("Feb", 20, 8), ("Mar", 35, 12), ("Apr", 50, 20), ("May", 100, 40)]
    frame1 = (
        '<draw:frame draw:z-index="0" draw:name="Object 1" draw:style-name="gr1" svg:width="16cm" svg:height="9cm" svg:x="0.3cm" svg:y="0.1cm" table:end-cell-address="Data.K21" table:end-x="1.2cm" table:end-y="0.4cm">'
        '<draw:object draw:notify-on-update-of-ranges="Data.A1:Data.C6" xlink:href="./Object 1" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"/>'
        '<draw:image xlink:href="./ObjectReplacements/Object 1" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"/></draw:frame>'
    )
    frame3 = (
        '<draw:frame draw:z-index="2" draw:name="Object 3" draw:style-name="gr1" svg:width="12cm" svg:height="8cm" svg:x="0cm" svg:y="0cm" table:end-cell-address="Data.J40" table:end-x="0cm" table:end-y="0cm">'
        '<draw:object xlink:href="./Object 3" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"/></draw:frame>'
    )
    frame_img = (
        '<draw:frame draw:z-index="3" draw:name="Image 1" draw:style-name="gr1" svg:width="2cm" svg:height="2cm" svg:x="0.5cm" svg:y="0.2cm" table:end-cell-address="Data.F3" table:end-x="0.2cm" table:end-y="0.1cm">'
        '<draw:image xlink:href="Pictures/10000000000000010000000149B5DCBA.png" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"><text:p/></draw:image></draw:frame>'
    )
    frame_math = (
        '<draw:frame draw:z-index="4" draw:name="Object 4" svg:width="2cm" svg:height="1cm" svg:x="0cm" svg:y="0cm">'
        '<draw:object xlink:href="./Object 4" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"/></draw:frame>'
    )
    rows = ['<table:table-row table:style-name="ro1">' + str_cell("Month") + str_cell("Sales") + str_cell("Costs") + '<table:table-cell table:number-columns-repeated="1021"/></table:table-row>']
    for i, (m, a, b) in enumerate(data):
        extra = ""
        if i == 0:
            # D2: the column chart (cell-anchored) and the picture in F2.
            extra = '<table:table-cell>%s</table:table-cell><table:table-cell/><table:table-cell>%s</table:table-cell><table:table-cell table:number-columns-repeated="1018"/>' % (frame1, frame_img)
        elif i == 1:
            extra = '<table:table-cell>%s%s</table:table-cell><table:table-cell table:number-columns-repeated="1020"/>' % (frame3, frame_math)
        else:
            extra = '<table:table-cell table:number-columns-repeated="1021"/>'
        rows.append('<table:table-row table:style-name="ro1">' + str_cell(m) + float_cell(a) + float_cell(b) + extra + "</table:table-row>")
    shapes = (
        '<table:shapes><draw:frame draw:z-index="1" draw:name="Object 2" draw:style-name="gr1" svg:width="8cm" svg:height="7cm" svg:x="9cm" svg:y="6cm">'
        '<draw:object xlink:href="./Object 2" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"/></draw:frame></table:shapes>'
    )
    table = (
        '<table:table table:name="Data" table:style-name="ta1">' + shapes
        + '<table:table-column table:style-name="co1" table:number-columns-repeated="1024" table:default-cell-style-name="Default"/>'
        + "".join(rows)
        + '<table:table-row table:style-name="ro1" table:number-rows-repeated="1048570"><table:table-cell table:number-columns-repeated="1024"/></table:table-row>'
        + "</table:table>"
    )
    extra_manifest = [
        ("Object 1/", "application/vnd.oasis.opendocument.chart"), ("Object 1/content.xml", "text/xml"), ("Object 1/styles.xml", "text/xml"),
        ("Object 2/", "application/vnd.oasis.opendocument.chart"), ("Object 2/content.xml", "text/xml"), ("Object 2/styles.xml", "text/xml"),
        ("Object 3/", "application/vnd.oasis.opendocument.chart"), ("Object 3/content.xml", "text/xml"), ("Object 3/styles.xml", "text/xml"),
        ("Object 4/", "application/vnd.oasis.opendocument.formula"), ("Object 4/content.xml", "text/xml"),
        ("ObjectReplacements/Object 1", "application/x-openoffice-gdimetafile;windows_formatname=\"GDIMetaFile\""),
        ("Pictures/10000000000000010000000149B5DCBA.png", "image/png"),
    ]
    man = [DECL, '<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.3">\n',
           ' <manifest:file-entry manifest:full-path="/" manifest:version="1.3" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/>\n']
    for f in ["content.xml", "styles.xml", "meta.xml", "settings.xml"]:
        man.append(' <manifest:file-entry manifest:full-path="%s" manifest:media-type="text/xml"/>\n' % f)
    for path, mt in extra_manifest:
        man.append(' <manifest:file-entry manifest:full-path="%s" manifest:media-type="%s"/>\n' % (path, mt.replace('"', "&quot;")))
    man.append("</manifest:manifest>\n")
    math = DECL + '<math xmlns="http://www.w3.org/1998/Math/MathML"><semantics><mi>x</mi></semantics></math>'
    write_zip(
        os.path.join(d, "libreoffice-charts.ods"),
        [
            ("META-INF/manifest.xml", "".join(man)),
            ("content.xml", content_doc(table, auto)),
            ("styles.xml", LO_STYLES),
            ("meta.xml", meta("LibreOffice/7.6.4.1$Linux_X86_64 LibreOffice_project/e19e193f88cd6c0525a17fb7a176ed8e6a3e2aa1")),
            ("settings.xml", settings("Data", [("Data", (0, 0, 0, 0))])),
            ("Object 1/content.xml", lo_column_chart()),
            ("Object 1/styles.xml", CHART_STYLES_XML),
            ("Object 2/content.xml", lo_pie_chart()),
            ("Object 2/styles.xml", CHART_STYLES_XML),
            ("Object 3/content.xml", lo_line_chart()),
            ("Object 3/styles.xml", CHART_STYLES_XML),
            ("Object 4/content.xml", math),
            ("ObjectReplacements/Object 1", b"VCLMTF" + b"\0" * 16),
            ("Pictures/10000000000000010000000149B5DCBA.png", PNG_1x1),
        ],
    )


# ---------------------------------------------------------------------------
# Data tools (LibreOffice 7.x shaped: named styles, outline groups, a
# scenario sheet, a named database range with a standard filter and
# subtotal rules, a DataPilot table; wave 5, W5-2)
# ---------------------------------------------------------------------------

TOOLS_STYLES = (
    DECL
    + '<office:document-styles%s office:version="1.3"><office:styles>' % NS
    + '<style:default-style style:family="table-cell"><style:paragraph-properties style:tab-stop-distance="1.25cm"/></style:default-style>'
    + '<style:style style:name="Default" style:family="table-cell"/>'
    + '<style:style style:name="Heading" style:family="table-cell" style:parent-style-name="Default"><style:table-cell-properties style:text-align-source="fix" style:repeat-content="false"/><style:paragraph-properties fo:text-align="center"/><style:text-properties fo:font-size="16pt" fo:font-style="italic" fo:font-weight="bold"/></style:style>'
    + '<style:style style:name="Heading_20_1" style:display-name="Heading 1" style:family="table-cell" style:parent-style-name="Heading"><style:table-cell-properties style:rotation-angle="90"/></style:style>'
    + '<style:style style:name="Result" style:family="table-cell" style:parent-style-name="Default"><style:text-properties fo:font-style="italic" style:text-underline-style="solid" style:text-underline-width="auto" style:text-underline-color="font-color" fo:font-weight="bold"/></style:style>'
    + '<style:style style:name="Money" style:family="table-cell" style:parent-style-name="Result"><style:table-cell-properties fo:background-color="#ffffcc" style:shrink-to-fit="true" style:diagonal-bl-tr="0.74pt solid #ff0000"/><style:paragraph-properties fo:margin-left="0.706cm"/><style:text-properties style:text-underline-style="solid" style:text-underline-type="double" style:text-underline-width="auto" style:text-underline-color="font-color"/></style:style>'
    + "</office:styles><office:automatic-styles/><office:master-styles/></office:document-styles>"
)

TOOLS_AUTO = (
    BASE_AUTO
    + '<style:style style:name="ta2" style:family="table" style:master-page-name="Default"><style:table-properties table:display="false" style:writing-mode="lr-tb"/></style:style>'
    + '<style:style style:name="ce1" style:family="table-cell" style:parent-style-name="Heading"><style:text-properties fo:color="#ff0000"/></style:style>'
    + '<style:style style:name="ce2" style:family="table-cell" style:parent-style-name="Money"/>'
    + '<style:style style:name="ce3" style:family="table-cell" style:parent-style-name="Default"><style:table-cell-properties style:rotation-angle="45" style:rotation-align="none" style:shrink-to-fit="true" style:diagonal-tl-br="0.74pt solid #000000"/></style:style>'
)


def tools_content():
    def r(cells, extra=""):
        return '<table:table-row table:style-name="ro1"%s>%s</table:table-row>' % (extra, "".join(cells))

    data = (
        '<table:table table:name="Data" table:style-name="ta1">'
        '<table:table-column table:style-name="co1" table:default-cell-style-name="Default"/>'
        '<table:table-column-group table:display="true">'
        '<table:table-column table:style-name="co1" table:number-columns-repeated="2" table:default-cell-style-name="Default"/>'
        "</table:table-column-group>"
        '<table:table-column table:style-name="co1" table:default-cell-style-name="Default"/>'
        + r([
            '<table:table-cell table:style-name="ce1" office:value-type="string" calcext:value-type="string"><text:p>Region</text:p></table:table-cell>',
            str_cell("Product"), str_cell("Qty"), str_cell("Amount"),
        ])
        + '<table:table-row-group table:display="true">'
        + r([str_cell("East"), str_cell("Apple"), float_cell(10), float_cell(100, "ce2")])
        + '<table:table-row-group table:display="false">'
        + r([str_cell("East"), str_cell("Pear"), float_cell(5), float_cell(60)], ' table:visibility="collapse"')
        + r([str_cell("East"), str_cell("Plum"), float_cell(2), float_cell(30)], ' table:visibility="collapse"')
        + "</table:table-row-group>"
        + r([str_cell("West"), str_cell("Apple"), float_cell(20), float_cell(200)])
        + "</table:table-row-group>"
        + r([str_cell("West"), str_cell("Pear"), float_cell(9), float_cell(110, "ce3")])
        + r([str_cell("West"), str_cell("Plum"), float_cell(3), float_cell(35)])
        + "</table:table>"
    )
    scenario = (
        '<table:table table:name="High" table:style-name="ta2">'
        '<table:scenario table:scenario-ranges="Data.C2:Data.C3" table:is-active="false" table:display-border="true" table:border-color="#0000ff" table:copy-back="true" table:copy-styles="false" table:copy-formulas="true" table:comment="what if" table:protected="false"/>'
        '<table:table-column table:style-name="co1" table:number-columns-repeated="3" table:default-cell-style-name="Default"/>'
        + r(['<table:table-cell table:number-columns-repeated="3"/>'])
        + r(['<table:table-cell table:number-columns-repeated="2"/>', float_cell(99)])
        + r(['<table:table-cell table:number-columns-repeated="2"/>', float_cell(55)])
        + "</table:table>"
    )
    pivot_sheet = (
        '<table:table table:name="Pivot" table:style-name="ta1">'
        '<table:table-column table:style-name="co1" table:number-columns-repeated="3" table:default-cell-style-name="Default"/>'
        + r([str_cell("Region"), str_cell("Sum - Amount")])
        + r([str_cell("East"), float_cell(190)])
        + r([str_cell("West"), float_cell(345)])
        + r([str_cell("Total Result"), float_cell(535)])
        + "</table:table>"
    )
    dbranges = (
        "<table:database-ranges>"
        '<table:database-range table:name="Sales" table:target-range-address="Data.A1:Data.D7" table:contains-header="true" table:display-filter-buttons="false">'
        '<table:filter table:display-duplicates="false"><table:filter-or>'
        '<table:filter-condition table:field-number="0" table:value="East" table:operator="=" table:data-type="text" table:case-sensitive="true"/>'
        '<table:filter-and>'
        '<table:filter-condition table:field-number="2" table:value="2" table:operator="top values" table:data-type="number"/>'
        '<table:filter-condition table:field-number="1" table:value="P" table:operator="begins-with" table:data-type="text"/>'
        "</table:filter-and>"
        "</table:filter-or></table:filter>"
        '<table:subtotal-rules table:case-sensitive="false" table:page-breaks-on-group-change="true" table:bind-styles-to-content="false">'
        '<table:sort-groups table:data-type="automatic" table:order="descending"/>'
        '<table:subtotal-rule table:group-by-field-number="0"><table:subtotal-field table:field-number="3" table:function="sum"/><table:subtotal-field table:field-number="2" table:function="average"/></table:subtotal-rule>'
        "</table:subtotal-rules>"
        "</table:database-range>"
        "</table:database-ranges>"
    )
    pivots = (
        "<table:data-pilot-tables>"
        '<table:data-pilot-table table:name="DataPilot1" table:application-data="" table:grand-total="both" table:show-filter-button="false" table:drill-down-on-double-click="true" table:target-range-address="Pivot.A1:Pivot.B4" table:ignore-empty-rows="true">'
        '<table:database-source-table table:database-name="" table:table-name="Sales"/>'
        '<table:data-pilot-field table:source-field-name="Region" table:orientation="row" table:used-hierarchy="1">'
        '<table:data-pilot-level table:show-empty="false">'
        '<table:data-pilot-subtotals><table:data-pilot-subtotal table:function="auto"/></table:data-pilot-subtotals>'
        '<table:data-pilot-members><table:data-pilot-member table:name="West" table:display="true" table:show-details="true"/><table:data-pilot-member table:name="East" table:display="true" table:show-details="true"/></table:data-pilot-members>'
        '<table:data-pilot-sort-info table:sort-mode="data" table:order="descending" table:data-field="Sum - Amount"/>'
        '<table:data-pilot-layout-info table:add-empty-lines="false" table:layout-mode="tabular-layout"/>'
        "</table:data-pilot-level></table:data-pilot-field>"
        '<table:data-pilot-field table:source-field-name="Product" table:orientation="page" table:selected-page="Apple" table:used-hierarchy="1"><table:data-pilot-level table:show-empty="false"><table:data-pilot-subtotals/><table:data-pilot-members><table:data-pilot-member table:name="Plum" table:display="false" table:show-details="true"/></table:data-pilot-members></table:data-pilot-level></table:data-pilot-field>'
        '<table:data-pilot-field table:source-field-name="Qty" table:orientation="column" table:used-hierarchy="1"><table:data-pilot-level table:show-empty="true"><table:data-pilot-subtotals><table:data-pilot-subtotal table:function="max"/><table:data-pilot-subtotal table:function="min"/></table:data-pilot-subtotals></table:data-pilot-level>'
        '<table:data-pilot-groups table:source-field-name="Qty" table:start="0" table:end="auto" table:step="5"/>'
        "</table:data-pilot-field>"
        '<table:data-pilot-field table:source-field-name="Amount" table:orientation="data" table:function="sum" table:used-hierarchy="1"><table:data-pilot-field-reference table:field-name="Region" table:type="row-percentage"/></table:data-pilot-field>'
        '<table:data-pilot-field table:source-field-name="Amount" table:orientation="data" table:function="count" table:used-hierarchy="1"/>'
        '<table:data-pilot-field table:source-field-name="" table:is-data-layout-field="true" table:orientation="column" table:used-hierarchy="1"/>'
        "</table:data-pilot-table>"
        "</table:data-pilot-tables>"
    )
    return content_doc(data + scenario + pivot_sheet + dbranges + pivots, TOOLS_AUTO)


def tools_settings():
    base = settings("Data", [("Data", (0, 0, 0, 0)), ("Pivot", (0, 0, 0, 0))])
    item = '<config:config-item config:name="ShowGrid" config:type="boolean">false</config:config-item><config:config-item config:name="ShowZeroValues" config:type="boolean">false</config:config-item>'
    return base.replace('<config:config-item-map-entry config:name="Data">', '<config:config-item-map-entry config:name="Data">' + item, 1)


def gen_tools():
    write_zip(
        os.path.join(ODS_DIR, "tools-lo.ods"),
        [
            ("META-INF/manifest.xml", manifest("1.3")),
            ("content.xml", tools_content()),
            ("styles.xml", TOOLS_STYLES),
            ("meta.xml", meta("LibreOffice/7.6.4.1$Linux_X86_64 LibreOffice_project/e19e193f88cd6c0525a17fb7a176ed8e6a3e2aa1")),
            ("settings.xml", tools_settings()),
        ],
    )


def main():
    for d in (ODS_DIR, CSV_DIR, BAD_DIR):
        os.makedirs(d, exist_ok=True)
    gen_libreoffice()
    gen_aoo()
    gen_legacy_and_minimal()
    gen_zip_bomb()
    gen_doctype()
    gen_path_traversal()
    gen_repeat_bomb()
    gen_deep_nesting()
    gen_lying_size()
    gen_many_entries()
    gen_not_ods()
    gen_csv()
    gen_charts()
    gen_tools()
    for d in (ODS_DIR, CSV_DIR, BAD_DIR, os.path.join(ODS_DIR, "charts")):
        for f in sorted(os.listdir(d)):
            if f.endswith(".py"):
                continue
            print("%-55s %10d" % (os.path.relpath(os.path.join(d, f), FIXTURES), os.path.getsize(os.path.join(d, f))))


if __name__ == "__main__":
    main()
