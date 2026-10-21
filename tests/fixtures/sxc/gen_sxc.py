#!/usr/bin/env python3
"""Generates the OpenOffice.org 1.x (.sxc) fixture for tally-io-ods.

Run with:  python3 -I tests/fixtures/sxc/gen_sxc.py

The XML follows the OOo 1.0 XML File Format specification: `office:`
namespaces of 2000, `table:value-type`/`table:value` on cells, native
formulas without a prefix, one `style:properties` element per style.
"""

import os
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
MIMETYPE = b"application/vnd.sun.xml.calc"
NS = (
    ' xmlns:office="http://openoffice.org/2000/office"'
    ' xmlns:style="http://openoffice.org/2000/style"'
    ' xmlns:text="http://openoffice.org/2000/text"'
    ' xmlns:table="http://openoffice.org/2000/table"'
    ' xmlns:fo="http://www.w3.org/1999/XSL/Format"'
    ' xmlns:number="http://openoffice.org/2000/datastyle"'
    ' xmlns:svg="http://www.w3.org/2000/svg"'
    ' office:class="spreadsheet" office:version="1.0"'
)

CONTENT = (
    '<?xml version="1.0" encoding="UTF-8"?>\n'
    '<office:document-content' + NS + '>'
    '<office:automatic-styles>'
    '<style:style style:name="co1" style:family="table-column"><style:properties style:column-width="3.5cm"/></style:style>'
    '<style:style style:name="ro1" style:family="table-row"><style:properties style:row-height="0.6cm" style:use-optimal-row-height="false"/></style:style>'
    '<style:style style:name="ce1" style:family="table-cell"><style:properties fo:font-weight="bold" fo:background-color="#ffff00" style:text-underline="single"/></style:style>'
    '</office:automatic-styles>'
    '<office:body>'
    '<table:table table:name="Sheet1">'
    '<table:table-column table:style-name="co1" table:number-columns-repeated="3"/>'
    '<table:table-row table:style-name="ro1">'
    '<table:table-cell table:value-type="float" table:value="1"><text:p>1</text:p></table:table-cell>'
    '<table:table-cell table:value-type="float" table:value="2"><text:p>2</text:p></table:table-cell>'
    '<table:table-cell table:value-type="string" table:style-name="ce1"><text:p>Total</text:p></table:table-cell>'
    '</table:table-row>'
    '<table:table-row>'
    '<table:table-cell table:formula="=SUM(A1:B1)" table:value-type="float" table:value="3"><text:p>3</text:p></table:table-cell>'
    '<table:table-cell table:formula="=A1*2" table:value-type="float" table:value="2"><text:p>2</text:p></table:table-cell>'
    '<table:table-cell table:formula="=Sheet2.A1&amp;&quot;!&quot;" table:value-type="string"><text:p>x!</text:p></table:table-cell>'
    '</table:table-row>'
    '<table:table-row>'
    '<table:table-cell table:value-type="date" table:date-value="2026-10-20"><text:p>2026-10-20</text:p></table:table-cell>'
    '<table:table-cell table:value-type="boolean" table:boolean-value="true"><text:p>TRUE</text:p></table:table-cell>'
    '<table:table-cell table:number-columns-spanned="1" table:number-rows-spanned="1"/>'
    '</table:table-row>'
    '</table:table>'
    '<table:table table:name="Sheet2">'
    '<table:table-row><table:table-cell table:value-type="string"><text:p>x</text:p></table:table-cell></table:table-row>'
    '</table:table>'
    '<table:named-expressions><table:named-range table:name="First" table:base-cell-address="Sheet1.A1" table:cell-range-address="Sheet1.A1:B1"/></table:named-expressions>'
    '</office:body></office:document-content>'
)

STYLES = (
    '<?xml version="1.0" encoding="UTF-8"?>\n'
    '<office:document-styles' + NS + '><office:styles>'
    '<style:default-style style:family="table-cell"><style:properties style:font-name="Albany" fo:language="de" fo:country="DE"/></style:default-style>'
    '</office:styles></office:document-styles>'
)

MANIFEST = (
    '<?xml version="1.0" encoding="UTF-8"?>\n'
    '<manifest:manifest xmlns:manifest="http://openoffice.org/2001/manifest">'
    '<manifest:file-entry manifest:media-type="application/vnd.sun.xml.calc" manifest:full-path="/"/>'
    '<manifest:file-entry manifest:media-type="text/xml" manifest:full-path="content.xml"/>'
    '<manifest:file-entry manifest:media-type="text/xml" manifest:full-path="styles.xml"/>'
    '</manifest:manifest>'
)


def main():
    path = os.path.join(HERE, "ooo1.sxc")
    with zipfile.ZipFile(path, "w") as z:
        zi = zipfile.ZipInfo("mimetype", date_time=(1980, 1, 1, 0, 0, 0))
        zi.compress_type = zipfile.ZIP_STORED
        z.writestr(zi, MIMETYPE)
        for name, data in [("content.xml", CONTENT), ("styles.xml", STYLES), ("META-INF/manifest.xml", MANIFEST)]:
            zi = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            zi.compress_type = zipfile.ZIP_DEFLATED
            z.writestr(zi, data.encode("utf-8"))
    print("wrote", path)


if __name__ == "__main__":
    main()
