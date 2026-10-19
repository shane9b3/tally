# OpenOffice Calc - Complete Spreadsheet Function Inventory

Derived from the Apache OpenOffice source tree (github.com/apache/openoffice, branch `trunk`), not from documentation. Ground-truth files:

- `main/sc/source/ui/src/scfuncs.src` - Function Wizard resources: one `Resource SC_OPCODE_*` per built-in function, with description, function group (`ID_FUNCTION_GRP_*`), parameter count (`VAR_ARGS` / `PAIRED_VAR_ARGS`), per-parameter optional flags, parameter names and descriptions.
- `main/formula/source/core/resource/core_resource.src` - English function names (`RID_STRLIST_FUNCTION_NAMES_ENGLISH`) and ODF OpenFormula names (`RID_STRLIST_FUNCTION_NAMES_ENGLISH_ODFF`).
- `main/formula/inc/formula/compiler.hrc` (numeric `SC_OPCODE_*` values), `main/formula/inc/formula/opcode.hxx` (`oc*` enum).
- `main/sc/source/core/tool/interpr4.cxx` (opcode -> `ScInterpreter::Sc*` dispatch), `interpr1.cxx`..`interpr6.cxx` (implementations; `MustHaveParamCount` min/max arg checks were cross-checked against the wizard flags - they agree for every function except FIXED), `parclass.cxx` (parameter classes Value/Reference/Array/ForceArray).
- `main/scaddins/source/analysis/` (`analysisadd.idl` signatures, `analysishelper.cxx` FUNCDATA table, `analysis.src` descriptions, `analysis_funcnames.src` names) and `main/scaddins/source/datefunc/` (`dateadd.idl`, `datefunc.cxx`, `datefunc.src`).
- `main/sc/source/core/tool/odffmap.cxx` - ODF names of add-in functions.

The machine-readable twin of this file is `calc-functions.json` (same data, plus parameter descriptions, parameter classes, opcode numbers).

## Conventions

- **Signature**: OpenOffice uses `;` as the function parameter separator (the ODF/OpenFormula and default en-US UI separator in AOO). `[x]` = optional. `x1; x2; ...` = variadic (repeatable). Paired variadics (SUMIFS etc.) repeat as pairs.
- **Types** (assigned from the interpreter code and parameter class): `number`, `integer` (number truncated with approxFloor), `logical` (number, 0 = FALSE), `text`, `date`/`time`/`datetime` (serial number, days since null date, default 1899-12-30), `any`, `reference` (must be a cell/range reference), `range` (reference or inline array), `array` (matrix; ForceArray class = argument is evaluated in array mode), `criteria` (number, text, or comparison expression like ">5", regex if enabled), `numberList` (numbers, ranges and arrays flattened; text/empty cells in ranges ignored), `valueList` (like numberList but text counts / evaluated as 0 - the *A functions), `logicalList`, `textList`.
- **Min/Max**: argument counts. Variadic functions accept up to 255 arguments (the parameter count is stored in a `sal_uInt8`), except where the interpreter enforces a lower limit (SUMPRODUCT 30, NPV 31, GETPIVOTDATA 30, CHOOSE 30 values via MAXJUMPCOUNT 32).
- **Opcode**: `SC_OPCODE_*` name and numeric value (compiler.hrc); add-in functions are all compiled as `ocExternal` with the UNO programmatic name.
- **Defaults** shown as `param: type=default` were read from the interpreter source.

## Summary counts

| Category | Count |
|---|---|
| Database | 12 |
| Date&Time | 17 |
| Financial | 22 |
| Information | 17 |
| Logical | 7 |
| Mathematical | 68 |
| Array | 14 |
| Statistical | 80 |
| Spreadsheet | 22 |
| Text | 37 |
| Add-in | 109 |
| **Built-in total** | **296** |
| Add-in: Analysis (`com.sun.star.sheet.addin.Analysis`) | 101 |
| Add-in: DateFunctions (`com.sun.star.sheet.addin.DateFunctions`) | 8 |
| **Overall total** | **405** |

Notes on the count: the built-in total includes MULTIPLE.OPERATIONS (no wizard entry, but a real function, ODF `MULTIPLE.OPERATIONS`) and NUMBERVALUE (declared but suppressed and unimplemented in AOO - see its entry). Excluding NUMBERVALUE gives 295 usable built-ins and 404 usable functions overall. Operators (`+ - * / ^ & = <> < > <= >= % ~ ! :` and unary `-`) and internal opcodes (GOALSEEK/ocBackSolver, MULTIRANGE, MVALUE, ocErrCell) are not functions and are listed in calc-features-inventory.md.

## Database (12)

| Name | Signature | Description | Param types (=default) | Returns | Min..Max | Opcode (id) / ODF name |
|---|---|---|---|---|---|---|
| DAVERAGE | `DAVERAGE(Database; Database_field; Search_criteria)` | Returns the average value of all the cells of a data range whose contents match the search criteria. | Database: range, Database field: any, Search criteria: range | number | 3..3 | `SC_OPCODE_DB_AVERAGE` (270) |
| DCOUNT | `DCOUNT(Database; Database_field; Search_criteria)` | Counts the cells of a data range whose contents match the search criteria. | Database: range, Database field: any, Search criteria: range | integer | 3..3 | `SC_OPCODE_DB_COUNT` (268) |
| DCOUNTA | `DCOUNTA(Database; Database_field; Search_criteria)` | Counts all non-blank cells of a data range where the content corresponds to the search criteria. | Database: range, Database field: any, Search criteria: range | integer | 3..3 | `SC_OPCODE_DB_COUNT_2` (269) |
| DGET | `DGET(Database; Database_field; Search_criteria)` | Defines the contents of the cell of a data range which matches the search criteria. | Database: range, Database field: any, Search criteria: range | any | 3..3 | `SC_OPCODE_DB_GET` (271) |
| DMAX | `DMAX(Database; Database_field; Search_criteria)` | Returns the maximum value from all of the cells of a data range which correspond to the search criteria. | Database: range, Database field: any, Search criteria: range | number | 3..3 | `SC_OPCODE_DB_MAX` (272) |
| DMIN | `DMIN(Database; Database_field; Search_criteria)` | Returns the minimum of all cells of a data range where the contents correspond to the search criteria. | Database: range, Database field: any, Search criteria: range | number | 3..3 | `SC_OPCODE_DB_MIN` (273) |
| DPRODUCT | `DPRODUCT(Database; Database_field; Search_criteria)` | Multiplies all cells of a data range where the contents match the search criteria. | Database: range, Database field: any, Search criteria: range | number | 3..3 | `SC_OPCODE_DB_PRODUCT` (274) |
| DSTDEV | `DSTDEV(Database; Database_field; Search_criteria)` | Calculates the standard deviation of all cells in a data range whose contents match the search criteria. | Database: range, Database field: any, Search criteria: range | number | 3..3 | `SC_OPCODE_DB_STD_DEV` (275) |
| DSTDEVP | `DSTDEVP(Database; Database_field; Search_criteria)` | Returns the standard deviation with regards to the population of all cells of a data range matching the search criteria. | Database: range, Database field: any, Search criteria: range | number | 3..3 | `SC_OPCODE_DB_STD_DEV_P` (276) |
| DSUM | `DSUM(Database; Database_field; Search_criteria)` | Adds all the cells of a data range where the contents match the search criteria. | Database: range, Database field: any, Search criteria: range | number | 3..3 | `SC_OPCODE_DB_SUM` (267) |
| DVAR | `DVAR(Database; Database_field; Search_criteria)` | Determines the variance of all the cells in a data range where the contents match the search criteria. | Database: range, Database field: any, Search criteria: range | number | 3..3 | `SC_OPCODE_DB_VAR` (277) |
| DVARP | `DVARP(Database; Database_field; Search_criteria)` | Determines variance of a population based on all cells in a data range where contents match the search criteria. | Database: range, Database field: any, Search criteria: range | number | 3..3 | `SC_OPCODE_DB_VAR_P` (278) |

## Date&Time (17)

| Name | Signature | Description | Param types (=default) | Returns | Min..Max | Opcode (id) / ODF name |
|---|---|---|---|---|---|---|
| DATE | `DATE(year; month; day)` | Provides an internal number for the date given. | year: integer, month: integer, day: integer | date | 3..3 | `SC_OPCODE_GET_DATE` (218) |
| DATEVALUE | `DATEVALUE(text)` | Returns an internal number for a text having a possible date format. | text: text | date | 1..1 | `SC_OPCODE_GET_DATE_VALUE` (131) |
| DAY | `DAY(Number)` | Returns the sequential date of the month as an integer (1-31) in relation to the date value. | Number: date | integer | 1..1 | `SC_OPCODE_GET_DAY` (108) |
| DAYS | `DAYS(Date_2; Date_1)` | Calculates the number of days between two dates. | Date_2: date, Date_1: date | integer | 2..2 | `SC_OPCODE_GET_DIFF_DATE` (220) |
| DAYS360 | `DAYS360(Date_1; Date_2; [Type])` | Calculates the number of days between two dates based on a 360-day year. | Date_1: date, Date_2: date, Type: integer=FALSE (US/NASD method) | integer | 2..3 | `SC_OPCODE_GET_DIFF_DATE_360` (221) |
| EASTERSUNDAY | `EASTERSUNDAY(year)` | Calculates the date of Easter Sunday in a given year. | year: integer | date | 1..1 | `SC_OPCODE_EASTERSUNDAY` (380) |
| HOUR | `HOUR(Number)` | Determines the sequential number of the hour of the day (0-23) for the time value. | Number: time | integer | 1..1 | `SC_OPCODE_GET_HOUR` (109) |
| MINUTE | `MINUTE(Number)` | Determines the sequential number for the minute of the hour (0-59) for the time value. | Number: time | integer | 1..1 | `SC_OPCODE_GET_MIN` (110) |
| MONTH | `MONTH(Number)` | Determines the sequential number of a month of the year (1-12) for the date value. | Number: date | integer | 1..1 | `SC_OPCODE_GET_MONTH` (107) |
| NOW | `NOW()` | Determines the current time of the computer. | - | datetime | 0..0 | `SC_OPCODE_GET_ACT_TIME` (70) |
| SECOND | `SECOND(Number)` | Determines the sequential number of the second of a minute (0-59) for the time value. | Number: time | integer | 1..1 | `SC_OPCODE_GET_SEC` (111) |
| TIME | `TIME(hour; minute; second)` | Determines a time value from the details for hour, minute and second. | hour: integer, minute: integer, second: integer | time | 3..3 | `SC_OPCODE_GET_TIME` (219) |
| TIMEVALUE | `TIMEVALUE(text)` | Returns a sequential number for a text shown in a possible time entry format. | text: text | time | 1..1 | `SC_OPCODE_GET_TIME_VALUE` (132) |
| TODAY | `TODAY()` | Determines the current date of the computer. | - | date | 0..0 | `SC_OPCODE_GET_ACT_DATE` (69) |
| WEEKDAY | `WEEKDAY(Number; [Type])` | Returns the day of the week for the date value as an integer (1-7). | Number: date, Type: integer=1 (Sunday=1..Saturday=7) | integer | 1..2 | `SC_OPCODE_GET_DAY_OF_WEEK` (366) |
| WEEKNUM | `WEEKNUM(Number; mode)` | Calculates the calendar week corresponding to the given date. | Number: date, mode: integer | integer | 2..2 | `SC_OPCODE_WEEK` (365) / ODF `ISOWEEKNUM` |
| YEAR | `YEAR(Number)` | Returns the year of a date value as an integer. | Number: date | integer | 1..1 | `SC_OPCODE_GET_YEAR` (106) |

## Financial (22)

| Name | Signature | Description | Param types (=default) | Returns | Min..Max | Opcode (id) / ODF name |
|---|---|---|---|---|---|---|
| CUMIPMT | `CUMIPMT(Rate; NPER; pv; S; E; Type)` | Cumulative compounded interest. Calculates the total amount of the interest share in a period for an investment with a constant interest rate. | Rate: number, NPER: number, pv: number, S: integer, E: integer, Type: integer | number | 6..6 | `SC_OPCODE_KUM_ZINS_Z` (262) |
| CUMPRINC | `CUMPRINC(Rate; NPER; PV; S; E; Type)` | Cumulative Capital. Calculates the total amount of the repayment share in a period for an investment with constant interest rate. | Rate: number, NPER: number, PV: number, S: integer, E: integer, Type: integer | number | 6..6 | `SC_OPCODE_KUM_KAP_Z` (263) |
| DB | `DB(Cost; Salvage; Life; Period; [month])` | Returns the real depreciation of an asset for a specified period using the fixed-declining balance method. | Cost: number, Salvage: number, Life: number, Period: number, month: integer=12 | number | 4..5 | `SC_OPCODE_GDA_2` (247) |
| DDB | `DDB(Cost; Salvage; Life; Period; [Factor])` | Calculates the depreciation of an asset for a specific period using the double-declining balance method or declining balance factor. | Cost: number, Salvage: number, Life: number, Period: number, Factor: number=2 | number | 4..5 | `SC_OPCODE_GDA` (246) |
| DURATION | `DURATION(RATE; pv; FV)` | Duration. Calculates the number of periods required by an investment to attain the desired value. | RATE: number, pv: number, FV: number | number | 3..3 | `SC_OPCODE_LAUFZ` (249) / ODF `PDURATION` |
| EFFECTIVE | `EFFECTIVE(NOM; P)` | Calculates the annual net interest rate for a nominal interest rate. | NOM: number, P: number | number | 2..2 | `SC_OPCODE_EFFEKTIV` (264) / ODF `EFFECT` |
| FV | `FV(Rate; NPER; PMT; [PV]; [Type])` | Future value. Returns the future value of an investment based on regular payments and a constant interest rate. | Rate: number, NPER: number, PMT: number, PV: number=0, Type: integer=0 | number | 3..5 | `SC_OPCODE_ZW` (257) |
| IPMT | `IPMT(Rate; Period; NPER; pv; [FV]; [Type])` | Compounded interest. Calculates the interest payment on the principal for an investment with regular payments and a constant interest rate for a given period. | Rate: number, Period: number, NPER: number, pv: number, FV: number=0, Type: integer=0 | number | 4..6 | `SC_OPCODE_ZINS_Z` (260) |
| IRR | `IRR(Values; [Guess])` | Returns the actuarial rate of interest of an investment excluding costs or profits. | Values: range, Guess: number=0.1 | number | 1..2 | `SC_OPCODE_IKV` (230) |
| ISPMT | `ISPMT(rate; Period; total_periods; invest)` | Returns the amount of interest for constant amortization rates. | rate: number, Period: number, total_periods: number, invest: number | number | 4..4 | `SC_OPCODE_ISPMT` (386) |
| MIRR | `MIRR(Values; investment; reinvest_rate)` | Returns the modified internal rate of return for a series of investments. | Values: range, investment: number, reinvest_rate: number | number | 3..3 | `SC_OPCODE_MIRR` (384) |
| NOMINAL | `NOMINAL(effect_rate; npery)` | Calculates the yearly nominal interest rate as an effective interest rate. | effect_rate: number, npery: number | number | 2..2 | `SC_OPCODE_NOMINAL` (265) |
| NPER | `NPER(Rate; PMT; PV; [FV]; [Type])` | Payment period. Calculates the number of payment periods for an investment based on regular payments and a constant interest rate. | Rate: number, PMT: number, PV: number, FV: number=0, Type: integer=0 | number | 3..5 | `SC_OPCODE_ZZR` (258) |
| NPV | `NPV(RATE; value1; value2; ...)` | Net present value. Calculates the net present value of an investment based on a series of periodic payments and a discount rate. | RATE: number, value: numberList | number | 2..31 | `SC_OPCODE_NBW` (229) |
| PMT | `PMT(Rate; NPER; PV; [FV]; [Type])` | Regular payments. Returns the periodic payment of an annuity, based on regular payments and a fixed periodic interest rate. | Rate: number, NPER: number, PV: number, FV: number=0, Type: integer=0 | number | 3..5 | `SC_OPCODE_RMZ` (251) |
| PPMT | `PPMT(Rate; Period; NPER; PV; [FV]; [Type])` | Repayment. Calculates the repayment amount for a period for an investment whereby the payments are at regular intervals and the interest rate constant. | Rate: number, Period: number, NPER: number, PV: number, FV: number=0, Type: integer=0 | number | 4..6 | `SC_OPCODE_KAPZ` (261) |
| PV | `PV(Rate; NPER; PMT; [FV]; [Type])` | Present value. Calculates the present value of an investment. | Rate: number, NPER: number, PMT: number, FV: number=0, Type: integer=0 | number | 3..5 | `SC_OPCODE_BW` (244) |
| RATE | `RATE(NPER; PMT; PV; [FV]; [Type]; [Guess])` | Calculates the constant interest rate of an investment with regular payments. | NPER: number, PMT: number, PV: number, FV: number=0, Type: integer=0, Guess: number=0.1 | number | 3..6 | `SC_OPCODE_ZINS` (259) |
| SLN | `SLN(Cost; Salvage; Life)` | Calculates the linear depreciation per period. | Cost: number, Salvage: number, Life: number | number | 3..3 | `SC_OPCODE_LIA` (250) |
| SYD | `SYD(Cost; Salvage; Life; Period)` | Calculates the arithmetically declining value of an asset (depreciation) for a specified period. | Cost: number, Salvage: number, Life: number, Period: number | number | 4..4 | `SC_OPCODE_DIA` (245) |
| VDB | `VDB(Cost; Salvage; Life; S; end; [Factor]; [Type])` | Variable declining balance. Returns the declining balance depreciation for a particular period. | Cost: number, Salvage: number, Life: number, S: number, end: number, Factor: number=2, Type: logical=FALSE | number | 5..7 | `SC_OPCODE_VBD` (248) |
| ZGZ | `ZGZ(P; pv; FV)` | Interest. Calculates the interest rate which represents the rate of return from an investment. | P: number, pv: number, FV: number | number | 3..3 | `SC_OPCODE_ZGZ` (256) |

## Information (17)

| Name | Signature | Description | Param types (=default) | Returns | Min..Max | Opcode (id) / ODF name |
|---|---|---|---|---|---|---|
| CELL | `CELL(info_type; [Reference])` | Determines information about address, formatting or contents of a cell. | info_type: text, Reference: reference=current cell | any | 1..2 | `SC_OPCODE_CELL` (385) |
| CURRENT | `CURRENT()` | Calculates the current value of the formula at the present location. | - | any | 0..0 | `SC_OPCODE_CURRENT` (72) / ODF `ORG.OPENOFFICE.CURRENT` |
| FORMULA | `FORMULA(Reference)` | Returns the formula of a formula cell. | Reference: reference | text | 1..1 | `SC_OPCODE_FORMULA` (153) |
| INFO | `INFO(Text)` | Returns information about the environment. | Text: text | any | 1..1 | `SC_OPCODE_INFO` (155) |
| ISBLANK | `ISBLANK(value)` | Returns TRUE if value refers to an empty cell. | value: any | logical | 1..1 | `SC_OPCODE_IS_EMPTY` (117) |
| ISERR | `ISERR(value)` | Returns TRUE if the value is an error value not equal to #N/A. | value: any | logical | 1..1 | `SC_OPCODE_IS_ERR` (126) |
| ISERROR | `ISERROR(value)` | Returns TRUE if the value is an error value. | value: any | logical | 1..1 | `SC_OPCODE_IS_ERROR` (127) |
| ISFORMULA | `ISFORMULA(reference)` | Returns TRUE if the cell is a formula cell. | reference: reference | logical | 1..1 | `SC_OPCODE_IS_FORMULA` (124) |
| ISLOGICAL | `ISLOGICAL(value)` | Returns TRUE if the value carries a logical number format. | value: any | logical | 1..1 | `SC_OPCODE_IS_LOGICAL` (120) |
| ISNA | `ISNA(value)` | Returns TRUE if value equals #N/A. | value: any | logical | 1..1 | `SC_OPCODE_IS_NV` (125) |
| ISNONTEXT | `ISNONTEXT(value)` | Returns TRUE if the value is not text. | value: any | logical | 1..1 | `SC_OPCODE_IS_NON_STRING` (119) |
| ISNUMBER | `ISNUMBER(value)` | Returns TRUE if value is a number. | value: any | logical | 1..1 | `SC_OPCODE_IS_VALUE` (123) |
| ISREF | `ISREF(value)` | Returns TRUE if value is a reference. | value: any | logical | 1..1 | `SC_OPCODE_IS_REF` (122) |
| ISTEXT | `ISTEXT(value)` | Returns TRUE if value is text. | value: any | logical | 1..1 | `SC_OPCODE_IS_STRING` (118) |
| N | `N(value)` | Converts a value to a number. | value: any | number | 1..1 | `SC_OPCODE_N` (130) |
| NA | `NA()` | Not available. Returns the error value #N/A. | - | error | 0..0 | `SC_OPCODE_NO_VALUE` (71) |
| TYPE | `TYPE(value)` | Returns the data type of a value (1 = number, 2 = text, 4 = Boolean value, 8 = formula, 16 = error value, 64 = array). | value: any | integer | 1..1 | `SC_OPCODE_TYPE` (121) |

## Logical (7)

| Name | Signature | Description | Param types (=default) | Returns | Min..Max | Opcode (id) / ODF name |
|---|---|---|---|---|---|---|
| AND | `AND(Logical_value1; Logical_value2; ...)` | Returns TRUE if all arguments are TRUE. | Logical value: logicalList | logical | 1..255 | `SC_OPCODE_AND` (52) |
| FALSE | `FALSE()` | Defines the logical value as FALSE. | - | logical | 0..0 | `SC_OPCODE_FALSE` (68) |
| IF | `IF(Test; [Then_value]; [Otherwise_value])` | Specifies a logical test to be performed. | Test: logical, Then_value: any=TRUE, Otherwise_value: any=FALSE | any | 1..3 | `SC_OPCODE_IF` (6) |
| NOT | `NOT(Logical_value)` | Reverses the value of the argument. | Logical value: logical | logical | 1..1 | `SC_OPCODE_NOT` (60) |
| OR | `OR(Logical_value1; Logical_value2; ...)` | Returns TRUE if an argument is TRUE. | Logical value: logicalList | logical | 1..255 | `SC_OPCODE_OR` (53) |
| TRUE | `TRUE()` | Returns the logical value TRUE. | - | logical | 0..0 | `SC_OPCODE_TRUE` (67) |
| XOR | `XOR(Logical_value1; Logical_value2; ...)` | Returns TRUE if exactly one of the arguments is TRUE. | Logical value: logicalList | logical | 1..255 | `SC_OPCODE_XOR` (396) |

## Mathematical (68)

| Name | Signature | Description | Param types (=default) | Returns | Min..Max | Opcode (id) / ODF name |
|---|---|---|---|---|---|---|
| ABS | `ABS(Number)` | Absolute value of a number. | Number: number | number | 1..1 | `SC_OPCODE_ABS` (113) |
| ACOS | `ACOS(Number)` | Returns the arccosine of a number. | Number: number | number | 1..1 | `SC_OPCODE_ARC_COS` (87) |
| ACOSH | `ACOSH(Number)` | Returns the inverse hyperbolic cosine of a number. | Number: number | number | 1..1 | `SC_OPCODE_ARC_COS_HYP` (95) |
| ACOT | `ACOT(Number)` | Returns the inverse cotangent of a number. | Number: number | number | 1..1 | `SC_OPCODE_ARC_COT` (89) |
| ACOTH | `ACOTH(Number)` | Returns the inverse hyperbolic cotangent of a number. | Number: number | number | 1..1 | `SC_OPCODE_ARC_COT_HYP` (97) |
| ASIN | `ASIN(Number)` | Returns the arcsine of a number. | Number: number | number | 1..1 | `SC_OPCODE_ARC_SIN` (86) |
| ASINH | `ASINH(Number)` | Returns the inverse hyperbolic sine of a number. | Number: number | number | 1..1 | `SC_OPCODE_ARC_SIN_HYP` (94) |
| ATAN | `ATAN(Number)` | Returns the arctangent of a number. | Number: number | number | 1..1 | `SC_OPCODE_ARC_TAN` (88) |
| ATAN2 | `ATAN2(number_x; number_y)` | Returns the arctangent for the specified coordinates. | number_x: number, number_y: number | number | 2..2 | `SC_OPCODE_ARC_TAN_2` (201) |
| ATANH | `ATANH(Number)` | Returns the inverse hyperbolic tangent of a number. | Number: number | number | 1..1 | `SC_OPCODE_ARC_TAN_HYP` (96) |
| AVERAGEIF | `AVERAGEIF(range; criteria; [average_range])` | Averages the arguments that meet the conditions. | range: range, criteria: criteria, average_range: range=range (param 1) | number | 2..3 | `SC_OPCODE_AVERAGE_IF` (395) |
| AVERAGEIFS | `AVERAGEIFS(average_range; range1; criteria1[; range2; criteria2; ...])` | Averages the value of the cells that meet multiple criteria in multiple ranges. | average_range: range, range: range, criteria: criteria | number | 3..255 | `SC_OPCODE_AVERAGE_IFS` (398) |
| BITAND | `BITAND(Number1; Number2)` | Returns the bitwise AND of two integers | Number1: integer, Number2: integer | integer | 2..2 | `SC_OPCODE_BITAND` (404) |
| BITLSHIFT | `BITLSHIFT(Number; Shift)` | Bitwise left shift of an integer value. | Number: integer, Shift: integer | integer | 2..2 | `SC_OPCODE_BITLSHIFT` (407) |
| BITOR | `BITOR(Number1; Number2)` | Returns the bitwise OR of two integers | Number1: integer, Number2: integer | integer | 2..2 | `SC_OPCODE_BITOR` (405) |
| BITRSHIFT | `BITRSHIFT(Number; Shift)` | Bitwise right shift of an integer value. | Number: integer, Shift: integer | integer | 2..2 | `SC_OPCODE_BITRSHIFT` (408) |
| BITXOR | `BITXOR(Number1; Number2)` | Returns the bitwise XOR of two integers | Number1: integer, Number2: integer | integer | 2..2 | `SC_OPCODE_BITXOR` (406) |
| CEILING | `CEILING(Number; Significance; [Mode])` | Rounds a number up to the nearest multiple of significance. | Number: number, Significance: number, Mode: logical=FALSE | number | 2..3 | `SC_OPCODE_CEIL` (202) |
| COMBIN | `COMBIN(number_1; number_2)` | Calculates the number of combinations for elements without repetition. | number_1: integer, number_2: integer | number | 2..2 | `SC_OPCODE_KOMBIN` (240) |
| COMBINA | `COMBINA(number_1; number_2)` | Calculates the number of combinations of elements including repetition. | number_1: integer, number_2: integer | number | 2..2 | `SC_OPCODE_KOMBIN_2` (241) |
| CONVERT | `CONVERT(value; text; text)` | Converts a value according to a conversion table in the configuration (calc.xcu). | value: number, text: text, text: text | number | 3..3 | `SC_OPCODE_CONVERT` (382) / ODF `ORG.OPENOFFICE.CONVERT` |
| COS | `COS(Number)` | Returns the cosine of a number. | Number: number | number | 1..1 | `SC_OPCODE_COS` (83) |
| COSH | `COSH(Number)` | Returns the hyperbolic cosine of a number. | Number: number | number | 1..1 | `SC_OPCODE_COS_HYP` (91) |
| COT | `COT(Number)` | Returns the cotangent of a number. | Number: number | number | 1..1 | `SC_OPCODE_COT` (85) |
| COTH | `COTH(Number)` | Returns the hyperbolic cotangent of a number. | Number: number | number | 1..1 | `SC_OPCODE_COT_HYP` (93) |
| COUNTBLANK | `COUNTBLANK(range)` | Counts the blank cells in a specified range. | range: range | integer | 1..1 | `SC_OPCODE_COUNT_EMPTY_CELLS` (282) |
| COUNTIF | `COUNTIF(range; criteria)` | Counts the arguments which meet the set conditions. | range: range, criteria: criteria | integer | 2..2 | `SC_OPCODE_COUNT_IF` (283) |
| COUNTIFS | `COUNTIFS(range1; criteria1[; range2; criteria2; ...])` | Counts the cells that meet multiple criteria in multiple ranges. | range: range, criteria: criteria | integer | 2..255 | `SC_OPCODE_COUNT_IFS` (399) |
| CSC | `CSC(Angle)` | Return the cosecant of an angle. CSC(x)=1/SIN(x) | Angle: number | number | 1..1 | `SC_OPCODE_COSECANT` (98) |
| CSCH | `CSCH(Angle)` | Return the hyperbolic cosecant of a hyperbolic angle. CSCH(x)=1/SINH(x) | Angle: number | number | 1..1 | `SC_OPCODE_COSECANT_HYP` (100) |
| DEGREES | `DEGREES(Number)` | Converts a radian to degrees | Number: number | number | 1..1 | `SC_OPCODE_DEG` (80) |
| EUROCONVERT | `EUROCONVERT(value; from_currency; to_currency; [full_precision]; [triangulation_precision])` | Converts a value from one to another Euro currency. | value: number, from_currency: text, to_currency: text, full_precision: logical=FALSE, triangulation_precision: integer=(no intermediate rounding) | number | 3..5 | `SC_OPCODE_EUROCONVERT` (391) |
| EVEN | `EVEN(Number)` | Rounds a positive number up and negative number down to the nearest even integer. | Number: number | number | 1..1 | `SC_OPCODE_EVEN` (144) |
| EXP | `EXP(Number)` | Calculates the exponent for basis e. | Number: number | number | 1..1 | `SC_OPCODE_EXP` (102) |
| FACT | `FACT(Number)` | Calculates the factorial of a number. | Number: integer | number | 1..1 | `SC_OPCODE_FACT` (105) |
| FLOOR | `FLOOR(Number; Significance; [Mode])` | Rounds number down to the nearest multiple of significance. | Number: number, Significance: number, Mode: logical=FALSE | number | 2..3 | `SC_OPCODE_FLOOR` (203) |
| GCD | `GCD(Integer1; Integer2; ...)` | Greatest Common Divisor | Integer: numberList | integer | 1..255 | `SC_OPCODE_GGT` (210) |
| INT | `INT(Number)` | Rounds a number down to the nearest integer. | Number: number | number | 1..1 | `SC_OPCODE_INT` (114) |
| ISEVEN | `ISEVEN(value)` | Returns TRUE if value is an even integer. | value: number | logical | 1..1 | `SC_OPCODE_IS_EVEN` (128) |
| ISODD | `ISODD(value)` | Returns TRUE if value is an odd integer. | value: number | logical | 1..1 | `SC_OPCODE_IS_ODD` (129) |
| LCM | `LCM(Integer1; Integer2; ...)` | Lowest common multiple | Integer: numberList | integer | 1..255 | `SC_OPCODE_KGV` (211) |
| LN | `LN(Number)` | Calculates the natural logarithm of a number. | Number: number | number | 1..1 | `SC_OPCODE_LN` (103) |
| LOG | `LOG(Number; [Base])` | Calculates the logarithm to any specified base. | Number: number, Base: number=10 | number | 1..2 | `SC_OPCODE_LOG` (208) |
| LOG10 | `LOG10(Number)` | Calculates the base-10 logarithm of a number. | Number: number | number | 1..1 | `SC_OPCODE_LOG10` (143) |
| MOD | `MOD(Dividend; Divisor)` | Calculates the remainder of a division. | Dividend: number, Divisor: number | number | 2..2 | `SC_OPCODE_MOD` (212) |
| ODD | `ODD(Number)` | Rounds a positive number up and negative number down to the nearest odd integer. | Number: number | number | 1..1 | `SC_OPCODE_ODD` (145) |
| PI | `PI()` | Returns the value of the number Pi. | - | number | 0..0 | `SC_OPCODE_PI` (65) |
| POWER | `POWER(Base; Exponent)` | Returns a^b, base a raised to the power of exponent b. | Base: number, Exponent: number | number | 2..2 | `SC_OPCODE_POWER` (209) |
| PRODUCT | `PRODUCT(Number1; Number2; ...)` | Multiplies the arguments. | Number: numberList | number | 1..255 | `SC_OPCODE_PRODUCT` (225) |
| RADIANS | `RADIANS(Number)` | Converts degrees to radians | Number: number | number | 1..1 | `SC_OPCODE_RAD` (81) |
| RAND | `RAND()` | Returns a random number between 0 and 1. | - | number | 0..0 | `SC_OPCODE_RANDOM` (66) |
| ROUND | `ROUND(number; [count])` | Rounds a number to a predefined accuracy. | number: number, count: integer=0 | number | 1..2 | `SC_OPCODE_ROUND` (204) |
| ROUNDDOWN | `ROUNDDOWN(number; [count])` | Rounds a number down to a predefined accuracy. | number: number, count: integer=0 | number | 1..2 | `SC_OPCODE_ROUND_DOWN` (206) |
| ROUNDUP | `ROUNDUP(number; [count])` | Rounds a number up to the predefined accuracy. | number: number, count: integer=0 | number | 1..2 | `SC_OPCODE_ROUND_UP` (205) |
| SEC | `SEC(Angle)` | Return the secant of an angle. SEC(x)=1/COS(x) | Angle: number | number | 1..1 | `SC_OPCODE_SECANT` (99) |
| SECH | `SECH(Angle)` | Return the hyperbolic secant of a hyperbolic angle. SECH(x)=1/COSH(x) | Angle: number | number | 1..1 | `SC_OPCODE_SECANT_HYP` (101) |
| SIGN | `SIGN(Number)` | Returns the algebraic sign of a number. | Number: number | integer | 1..1 | `SC_OPCODE_PLUS_MINUS` (112) |
| SIN | `SIN(number)` | Returns the sine of a number. | number: number | number | 1..1 | `SC_OPCODE_SIN` (82) |
| SINH | `SINH(number)` | Returns the hyperbolic sine of a number. | number: number | number | 1..1 | `SC_OPCODE_SIN_HYP` (90) |
| SQRT | `SQRT(number)` | Returns the square root of a number. | number: number | number | 1..1 | `SC_OPCODE_SQRT` (104) |
| SUBTOTAL | `SUBTOTAL(Function; range)` | Calculates subtotals in a spreadsheet. | Function: integer, range: reference | number | 2..2 | `SC_OPCODE_SUB_TOTAL` (266) |
| SUM | `SUM(number1; number2; ...)` | Returns the sum of all arguments. | number: numberList | number | 1..255 | `SC_OPCODE_SUM` (224) |
| SUMIF | `SUMIF(range; criteria; [sum_range])` | Totals the arguments that meet the conditions. | range: range, criteria: criteria, sum_range: range=range (param 1) | number | 2..3 | `SC_OPCODE_SUM_IF` (284) |
| SUMIFS | `SUMIFS(sum_range; range1; criteria1[; range2; criteria2; ...])` | Totals the values of cells in a range that meet multiple criteria in multiple ranges. | sum_range: range, range: range, criteria: criteria | number | 3..255 | `SC_OPCODE_SUM_IFS` (397) |
| SUMSQ | `SUMSQ(number1; number2; ...)` | Returns the sum of the squares of the arguments. | number: numberList | number | 1..255 | `SC_OPCODE_SUM_SQ` (214) |
| TAN | `TAN(number)` | Returns the tangent of a number. | number: number | number | 1..1 | `SC_OPCODE_TAN` (84) |
| TANH | `TANH(number)` | Returns the hyperbolic tangent of a number. | number: number | number | 1..1 | `SC_OPCODE_TAN_HYP` (92) |
| TRUNC | `TRUNC(number; count)` | Truncates the decimal places of a number. | number: number, count: integer | number | 2..2 | `SC_OPCODE_TRUNC` (207) |

## Array (14)

| Name | Signature | Description | Param types (=default) | Returns | Min..Max | Opcode (id) / ODF name |
|---|---|---|---|---|---|---|
| FREQUENCY | `FREQUENCY(data; classes)` | Returns a frequency distribution as a vertical array. | data: array, classes: array | array | 2..2 | `SC_OPCODE_FREQUENCY` (336) |
| GROWTH | `GROWTH(data_Y; [data_X]; [new_data_X]; [Function_type])` | Calculates points on the exponential regression function. | data_Y: array, data_X: array={1;2;3;...}, new_data_X: array=data_X, Function_type: logical=TRUE | array | 1..4 | `SC_OPCODE_GROWTH` (351) |
| LINEST | `LINEST(data_Y; [data_X]; [Linear_type]; [stats])` | Calculates parameters of the linear regression as an array. | data_Y: array, data_X: array={1;2;3;...}, Linear_type: logical=TRUE, stats: logical=FALSE | array | 1..4 | `SC_OPCODE_RGP` (352) |
| LOGEST | `LOGEST(data_Y; [data_X]; [Function_type]; [stats])` | Calculates the parameters of the exponential regression curve as an array. | data_Y: array, data_X: array={1;2;3;...}, Function_type: logical=TRUE, stats: logical=FALSE | array | 1..4 | `SC_OPCODE_RKP` (353) |
| MDETERM | `MDETERM(array)` | Returns the array determinant. | array: array | number | 1..1 | `SC_OPCODE_MAT_DET` (306) |
| MINVERSE | `MINVERSE(array)` | Returns the inverse of an array. | array: array | array | 1..1 | `SC_OPCODE_MAT_INV` (307) |
| MMULT | `MMULT(array; array)` | Array multiplication. Returns the product of two arrays. | array: array, array: array | array | 2..2 | `SC_OPCODE_MAT_MULT` (308) |
| MUNIT | `MUNIT(Dimensions)` | Returns the unitary square array of a certain size. | Dimensions: integer | array | 1..1 | `SC_OPCODE_MATRIX_UNIT` (310) |
| SUMPRODUCT | `SUMPRODUCT(Array1; Array2; ...)` | (Inner products) Returns the sum of the products of array arguments. | Array: array | number | 1..30 | `SC_OPCODE_SUM_PRODUCT` (213) |
| SUMX2MY2 | `SUMX2MY2(array_x; array_y)` | Returns the sum of the difference of squares of two arrays. | array_x: array, array_y: array | number | 2..2 | `SC_OPCODE_SUM_X2MY2` (215) |
| SUMX2PY2 | `SUMX2PY2(array_x; array_y)` | Returns the total of the square sum of two arrays. | array_x: array, array_y: array | number | 2..2 | `SC_OPCODE_SUM_X2DY2` (216) |
| SUMXMY2 | `SUMXMY2(array_x; array_y)` | Returns the sum of squares of differences of two arrays. | array_x: array, array_y: array | number | 2..2 | `SC_OPCODE_SUM_XMY2` (217) |
| TRANSPOSE | `TRANSPOSE(array)` | Array transposition. Exchanges the rows and columns of an array. | array: array | array | 1..1 | `SC_OPCODE_MAT_TRANS` (309) |
| TREND | `TREND(data_Y; [data_X]; [new_data_X]; [Linear_type])` | Calculates points along a regression line. | data_Y: array, data_X: array={1;2;3;...}, new data_X: array=data_X, Linear_type: logical=TRUE | array | 1..4 | `SC_OPCODE_TREND` (350) |

## Statistical (80)

| Name | Signature | Description | Param types (=default) | Returns | Min..Max | Opcode (id) / ODF name |
|---|---|---|---|---|---|---|
| AVEDEV | `AVEDEV(number1; number2; ...)` | Returns the average of the absolute deviations of a sample from the mean. | number: numberList | number | 1..255 | `SC_OPCODE_AVE_DEV` (324) |
| AVERAGE | `AVERAGE(number1; number2; ...)` | Returns the average of a sample. | number: numberList | number | 1..255 | `SC_OPCODE_AVERAGE` (226) |
| AVERAGEA | `AVERAGEA(value1; value2; ...)` | Returns the average value for a sample. Text is evaluated as zero. | value: valueList | number | 1..255 | `SC_OPCODE_AVERAGE_A` (375) |
| B | `B(trials; SP; T_1; [T_2])` | Returns the probability of a trial result using binomial distribution. | trials: integer, SP: number, T_1: integer, T_2: integer=T_1 (point probability) | number | 3..4 | `SC_OPCODE_B` (235) |
| BETADIST | `BETADIST(number; alpha; beta; [Start]; [End]; [Cumulative])` | Values of the beta distribution. | number: number, alpha: number, beta: number, Start: number=0, End: number=1, Cumulative: logical=TRUE | number | 3..6 | `SC_OPCODE_BETA_DIST` (363) |
| BETAINV | `BETAINV(number; alpha; beta; [Start]; [End])` | Values of the inverse beta distribution. | number: number, alpha: number, beta: number, Start: number=0, End: number=1 | number | 3..5 | `SC_OPCODE_BETA_INV` (364) |
| BINOMDIST | `BINOMDIST(X; trials; SP; C)` | Values of the binomial distribution. | X: integer, trials: integer, SP: number, C: logical | number | 4..4 | `SC_OPCODE_BINOM_DIST` (238) |
| CHIDIST | `CHIDIST(Number; degrees_freedom)` | Returns the right-tail probability of the chi-square distribution. | Number: number, degrees_freedom: number | number | 2..2 | `SC_OPCODE_CHI_DIST` (316) / ODF `LEGACY.CHIDIST` |
| CHIINV | `CHIINV(number; degrees_freedom)` | Values of the inverse of CHIDIST(x; DegreesOfFreedom). | number: number, degrees_freedom: number | number | 2..2 | `SC_OPCODE_CHI_INV` (355) / ODF `LEGACY.CHIINV` |
| CHISQDIST | `CHISQDIST(Number; Degrees_of_Freedom; [Cumulative])` | Returns left-tail probability of the cumulative distribution function or values of the probability density function of the chi-square distribution. | Number: number, Degrees of Freedom: number, Cumulative: logical=TRUE | number | 2..3 | `SC_OPCODE_CHISQ_DIST` (393) |
| CHISQINV | `CHISQINV(Probability; Degrees_of_Freedom)` | Values of the inverse of CHISQDIST(x;DegreesOfFreedom;TRUE()). | Probability: number, Degrees of Freedom: number | number | 2..2 | `SC_OPCODE_CHISQ_INV` (394) |
| CHITEST | `CHITEST(Data_B; data_E)` | Returns the chi square independence test. | Data_B: array, data_E: array | number | 2..2 | `SC_OPCODE_CHI_TEST` (360) / ODF `LEGACY.CHITEST` |
| CONFIDENCE | `CONFIDENCE(alpha; STDEV; size)` | Returns a (1 alpha) confidence interval for a normal distribution. | alpha: number, STDEV: number, size: integer | number | 3..3 | `SC_OPCODE_CONFIDENCE` (339) |
| CORREL | `CORREL(Data_1; Data_2)` | Returns the correlation coefficient. | Data_1: array, Data_2: array | number | 2..2 | `SC_OPCODE_CORREL` (343) |
| COUNT | `COUNT(value1; value2; ...)` | Counts how many numbers are in the list of arguments. | value: valueList | integer | 1..255 | `SC_OPCODE_COUNT` (227) |
| COUNTA | `COUNTA(value1; value2; ...)` | Counts how many values are in the list of arguments. | value: valueList | integer | 1..255 | `SC_OPCODE_COUNT_2` (228) |
| COVAR | `COVAR(Data_1; Data_2)` | Calculates the covariance. | Data_1: array, Data_2: array | number | 2..2 | `SC_OPCODE_COVAR` (344) |
| CRITBINOM | `CRITBINOM(trials; SP; alpha)` | Border arguments of the binomial distribution. | trials: integer, SP: number, alpha: number | integer | 3..3 | `SC_OPCODE_KRIT_BINOM` (319) |
| DEVSQ | `DEVSQ(number1; number2; ...)` | Returns the sum of squares of deviations from the sample mean value | number: numberList | number | 1..255 | `SC_OPCODE_DEV_SQ` (326) |
| EXPONDIST | `EXPONDIST(Number; lambda; C)` | Values of the exponential distribution. | Number: number, lambda: number, C: logical | number | 3..3 | `SC_OPCODE_EXP_DIST` (237) |
| FDIST | `FDIST(Number; degrees_freedom_1; degrees_freedom_2)` | Values of the F probability distribution. | Number: number, degrees_freedom_1: number, degrees_freedom_2: number | number | 3..3 | `SC_OPCODE_F_DIST` (315) / ODF `LEGACY.FDIST` |
| FINV | `FINV(number; degrees_freedom_1; degrees_freedom_2)` | Values of the inverse F distribution. | number: number, degrees_freedom_1: number, degrees_freedom_2: number | number | 3..3 | `SC_OPCODE_F_INV` (359) / ODF `LEGACY.FINV` |
| FISHER | `FISHER(Number)` | Returns the Fisher transformation. | Number: number | number | 1..1 | `SC_OPCODE_FISHER` (147) |
| FISHERINV | `FISHERINV(Number)` | Returns the inverse of the Fisher transformation. | Number: number | number | 1..1 | `SC_OPCODE_FISHER_INV` (148) |
| FORECAST | `FORECAST(value; data_Y; data_X)` | Returns a value along a linear regression | value: number, data_Y: array, data_X: array | number | 3..3 | `SC_OPCODE_FORECAST` (354) |
| FTEST | `FTEST(data_1; data_2)` | Calculates the F test. | data_1: array, data_2: array | number | 2..2 | `SC_OPCODE_F_TEST` (340) |
| GAMMA | `GAMMA(Number)` | Returns the value of the Gamma function. | Number: number | number | 1..1 | `SC_OPCODE_GAMMA` (161) |
| GAMMADIST | `GAMMADIST(Number; alpha; beta; [Cumulative])` | Returns the value of the probability density function or the cumulative distribution function for the Gamma distribution. | Number: number, alpha: number, beta: number, Cumulative: logical=TRUE | number | 3..4 | `SC_OPCODE_GAMMA_DIST` (356) |
| GAMMAINV | `GAMMAINV(Number; alpha; beta)` | Values of the inverse gamma distribution. | Number: number, alpha: number, beta: number | number | 3..3 | `SC_OPCODE_GAMMA_INV` (357) |
| GAMMALN | `GAMMALN(Number)` | Returns the natural logarithm of the gamma function. | Number: number | number | 1..1 | `SC_OPCODE_GAMMA_LN` (150) |
| GAUSS | `GAUSS(Number)` | Returns the integral values of the standard normal cumulative distribution. | Number: number | number | 1..1 | `SC_OPCODE_GAUSS` (116) |
| GEOMEAN | `GEOMEAN(number1; number2; ...)` | Returns the geometric mean of a sample. | number: numberList | number | 1..255 | `SC_OPCODE_GEO_MEAN` (322) |
| HARMEAN | `HARMEAN(number1; number2; ...)` | Returns the harmonic mean of a sample. | number: numberList | number | 1..255 | `SC_OPCODE_HAR_MEAN` (321) |
| HYPGEOMDIST | `HYPGEOMDIST(X; n_sample; successes; n_population)` | Values of the hypergeometric distribution. | X: integer, n_sample: integer, successes: integer, n_population: integer | number | 4..4 | `SC_OPCODE_HYP_GEOM_DIST` (312) |
| INTERCEPT | `INTERCEPT(data_Y; data_X)` | Returns the intercept of the linear regression line and the Y axis. | data_Y: array, data_X: array | number | 2..2 | `SC_OPCODE_INTERCEPT` (349) |
| KURT | `KURT(number1; number2; ...)` | Returns the kurtosis of a distribution. | number: numberList | number | 1..255 | `SC_OPCODE_KURT` (320) |
| LARGE | `LARGE(data; Rank_c)` | Returns the k-th largest value of a sample. | data: array, Rank_c: integer | number | 2..2 | `SC_OPCODE_LARGE` (334) |
| LOGINV | `LOGINV(number; mean; STDEV)` | Values of the inverse of the lognormal distribution. | number: number, mean: number, STDEV: number | number | 3..3 | `SC_OPCODE_LOG_INV` (361) |
| LOGNORMDIST | `LOGNORMDIST(Number; [mean]; [STDEV]; [Cumulative])` | Values of the log normal distribution. | Number: number, mean: number=0, STDEV: number=1, Cumulative: logical=TRUE | number | 1..4 | `SC_OPCODE_LOG_NORM_DIST` (313) |
| MAX | `MAX(number1; number2; ...)` | Returns the maximum value in a list of arguments. | number: numberList | number | 1..255 | `SC_OPCODE_MAX` (223) |
| MAXA | `MAXA(value1; value2; ...)` | Returns the maximum value in a list of arguments. Text is evaluated as Zero. | value: valueList | number | 1..255 | `SC_OPCODE_MAX_A` (374) |
| MEDIAN | `MEDIAN(number1; number2; ...)` | Returns the median of a given sample. | number: numberList | number | 1..255 | `SC_OPCODE_MEDIAN` (327) |
| MIN | `MIN(number1; number2; ...)` | Returns the minimum value in a list of arguments. | number: numberList | number | 1..255 | `SC_OPCODE_MIN` (222) |
| MINA | `MINA(value1; value2; ...)` | Returns the smallest value in a list of arguments. Text is evaluated as zero. | value: valueList | number | 1..255 | `SC_OPCODE_MIN_A` (373) |
| MODE | `MODE(number1; number2; ...)` | Returns the most common value in a sample. | number: numberList | number | 1..255 | `SC_OPCODE_MODAL_VALUE` (328) |
| NEGBINOMDIST | `NEGBINOMDIST(X; R; SP)` | Values of the negative binomial distribution. | X: integer, R: integer, SP: number | number | 3..3 | `SC_OPCODE_NEG_BINOM_VERT` (318) |
| NORMDIST | `NORMDIST(Number; Mean; STDEV; [C])` | Values of the normal distribution. | Number: number, Mean: number, STDEV: number, C: logical=TRUE | number | 3..4 | `SC_OPCODE_NORM_DIST` (236) |
| NORMINV | `NORMINV(number; mean; STDEV)` | Values of the inverse normal distribution. | number: number, mean: number, STDEV: number | number | 3..3 | `SC_OPCODE_NORM_INV` (338) |
| NORMSDIST | `NORMSDIST(Number)` | The values of the standard normal cumulative distribution. | Number: number | number | 1..1 | `SC_OPCODE_STD_NORM_DIST` (146) / ODF `LEGACY.NORMSDIST` |
| NORMSINV | `NORMSINV(number)` | Values of the inverse standard normal distribution. | number: number | number | 1..1 | `SC_OPCODE_S_NORM_INV` (149) / ODF `LEGACY.NORMSINV` |
| PEARSON | `PEARSON(Data_1; Data_2)` | Returns the Pearson product moment correlation coefficient. | Data_1: array, Data_2: array | number | 2..2 | `SC_OPCODE_PEARSON` (345) |
| PERCENTILE | `PERCENTILE(data; Alpha)` | Returns the alpha quantile of a sample. | data: array, Alpha: number | number | 2..2 | `SC_OPCODE_PERCENTILE` (332) |
| PERCENTRANK | `PERCENTRANK(data; value)` | Returns the percentage rank of a value in a sample. | data: array, value: number | number | 2..2 | `SC_OPCODE_PERCENT_RANK` (333) |
| PERMUT | `PERMUT(Count_1; Count_2)` | Returns the number of permutations for a given number of elements without repetition. | Count_1: integer, Count_2: integer | number | 2..2 | `SC_OPCODE_VARIATIONEN` (242) |
| PERMUTATIONA | `PERMUTATIONA(Count_1; Count_2)` | Returns the number of permutations for a given number of objects (repetition allowed). | Count_1: integer, Count_2: integer | number | 2..2 | `SC_OPCODE_VARIATIONEN_2` (243) |
| PHI | `PHI(number)` | Values of the distribution function for a standard normal distribution. | number: number | number | 1..1 | `SC_OPCODE_PHI` (115) |
| POISSON | `POISSON(Number; mean; [Cumulative])` | Returns the Poisson distribution. | Number: integer, mean: number, Cumulative: logical=TRUE | number | 2..3 | `SC_OPCODE_POISSON_DIST` (239) |
| PROB | `PROB(data; probability; Start; [End])` | Returns the discrete probability of an interval. | data: array, probability: array, Start: number, End: number=Start (point probability) | number | 3..4 | `SC_OPCODE_PROB` (342) |
| QUARTILE | `QUARTILE(data; Type)` | Returns the quartile of a sample. | data: array, Type: integer | number | 2..2 | `SC_OPCODE_QUARTILE` (337) |
| RANK | `RANK(value; Data; [Type])` | Returns the ranking of a value in a sample. | value: number, Data: range, Type: integer=0 (descending) | integer | 2..3 | `SC_OPCODE_RANK` (331) |
| RSQ | `RSQ(data_Y; data_X)` | Returns the square of the Pearson product moment correlation coefficient. | data_Y: array, data_X: array | number | 2..2 | `SC_OPCODE_RSQ` (346) |
| SKEW | `SKEW(number1; number2; ...)` | Returns the skewness of a distribution. | number: numberList | number | 1..255 | `SC_OPCODE_SCHIEFE` (325) |
| SLOPE | `SLOPE(data_Y; data_X)` | Returns the slope of the linear regression line. | data_Y: array, data_X: array | number | 2..2 | `SC_OPCODE_SLOPE` (348) |
| SMALL | `SMALL(data; Rank_c)` | Returns the k-th smallest value of a sample. | data: array, Rank_c: integer | number | 2..2 | `SC_OPCODE_SMALL` (335) |
| STANDARDIZE | `STANDARDIZE(Number; mean; STDEV)` | Converts a random variable to a normalized value. | Number: number, mean: number, STDEV: number | number | 3..3 | `SC_OPCODE_STANDARD` (323) |
| STDEV | `STDEV(number1; number2; ...)` | Calculates the standard deviation based on a sample. | number: numberList | number | 1..255 | `SC_OPCODE_ST_DEV` (233) |
| STDEVA | `STDEVA(value1; value2; ...)` | Returns the standard deviation based on a sample. Text is evaluated as zero. | value: valueList | number | 1..255 | `SC_OPCODE_ST_DEV_A` (376) |
| STDEVP | `STDEVP(number1; number2; ...)` | Calculates the standard deviation based on the entire population. | number: numberList | number | 1..255 | `SC_OPCODE_ST_DEV_P` (234) |
| STDEVPA | `STDEVPA(value1; value2; ...)` | Returns the standard deviation based on the entire population. Text is evaluated as zero. | value: valueList | number | 1..255 | `SC_OPCODE_ST_DEV_P_A` (377) |
| STEYX | `STEYX(data_Y; data_X)` | Returns the standard error of the linear regression. | data_Y: array, data_X: array | number | 2..2 | `SC_OPCODE_STEYX` (347) |
| TDIST | `TDIST(Number; degrees_freedom; mode)` | Returns the t-distribution. | Number: number, degrees_freedom: number, mode: integer | number | 3..3 | `SC_OPCODE_T_DIST` (314) |
| TINV | `TINV(number; degrees_freedom)` | Values of the inverse t-distribution. | number: number, degrees_freedom: number | number | 2..2 | `SC_OPCODE_T_INV` (358) |
| TRIMMEAN | `TRIMMEAN(data; Alpha)` | Returns the mean of a sample without including the marginal values. | data: array, Alpha: number | number | 2..2 | `SC_OPCODE_TRIM_MEAN` (341) |
| TTEST | `TTEST(data_1; data_2; mode; Type)` | Calculates the T test. | data_1: array, data_2: array, mode: integer, Type: integer | number | 4..4 | `SC_OPCODE_T_TEST` (330) |
| VAR | `VAR(number1; number2; ...)` | Calculates the variance based on a sample. | number: numberList | number | 1..255 | `SC_OPCODE_VAR` (231) |
| VARA | `VARA(value1; value2; ...)` | Returns the variance based on a sample. Text is evaluated as zero. | value: valueList | number | 1..255 | `SC_OPCODE_VAR_A` (378) |
| VARP | `VARP(number1; number2; ...)` | Calculates variance based on the entire population. | number: numberList | number | 1..255 | `SC_OPCODE_VAR_P` (232) |
| VARPA | `VARPA(value1; value2; ...)` | Returns the variance based on the entire population. Text is evaluated as zero. | value: valueList | number | 1..255 | `SC_OPCODE_VAR_P_A` (379) |
| WEIBULL | `WEIBULL(Number; Alpha; beta; C)` | Returns the values of the Weibull distribution. | Number: number, Alpha: number, beta: number, C: logical | number | 4..4 | `SC_OPCODE_WEIBULL` (317) |
| ZTEST | `ZTEST(data; mu; [sigma])` | Calculates the probability of observing a z-statistic greater than the one computed based on a sample. | data: array, mu: number, sigma: number=sample standard deviation | number | 2..3 | `SC_OPCODE_Z_TEST` (329) |

## Spreadsheet (22)

| Name | Signature | Description | Param types (=default) | Returns | Min..Max | Opcode (id) / ODF name |
|---|---|---|---|---|---|---|
| ADDRESS | `ADDRESS(row; column; [ABS]; [A1]; [sheet])` | Returns the reference to a cell as text. | row: integer, column: integer, ABS: integer=1 (absolute), A1: logical=TRUE (A1), sheet: text=(no sheet) | text | 2..5 | `SC_OPCODE_ADDRESS` (280) |
| AREAS | `AREAS(reference)` | Returns the number of individual ranges that belong to a (multiple) range. | reference: reference | integer | 1..1 | `SC_OPCODE_AREAS` (291) |
| CHOOSE | `CHOOSE(Index; value1; value2; ...)` | Selects a value from a list of up to 30 value arguments. | Index: integer, value: any | any | 2..31 | `SC_OPCODE_CHOSE` (7) |
| COLUMN | `COLUMN([reference])` | Returns the internal column number of a reference. | reference: reference=formula cell | integer | 0..1 | `SC_OPCODE_COLUMN` (254) |
| COLUMNS | `COLUMNS(array)` | Returns the number of columns in an array or reference. | array: range | integer | 1..1 | `SC_OPCODE_COLUMNS` (252) |
| DDE | `DDE(server; File; range; [mode])` | Result of a DDE link. | server: text, File: text, range: text, mode: integer=0 | any | 3..4 | `SC_OPCODE_DDE` (369) |
| ERRORTYPE | `ERRORTYPE(reference)` | Returns a number corresponding to an error type | reference: reference | integer | 1..1 | `SC_OPCODE_ERROR_TYPE` (151) / ODF `ORG.OPENOFFICE.ERRORTYPE` |
| GETPIVOTDATA | `GETPIVOTDATA(Data_Field; Pivot_Table; [Field_Name_/_Item1; Field_Name_/_Item2; ...])` | Extracts value(s) from a pivot table. | Data Field: text, Pivot Table: reference, Field Name / Item: any | any | 2..30 | `SC_OPCODE_GET_PIVOT_DATA` (390) |
| HLOOKUP | `HLOOKUP(search_criteria; array; Index; [sorted])` | Horizontal search and reference to the cells located below. | search_criteria: any, array: range, Index: integer, sorted: logical=TRUE | any | 3..4 | `SC_OPCODE_H_LOOKUP` (287) |
| HYPERLINK | `HYPERLINK(URL; [CellText])` | Hyperlink. | URL: text, CellText: any=URL | any | 1..2 | `SC_OPCODE_HYPERLINK` (387) |
| INDEX | `INDEX(reference; [row]; [column]; [range])` | Returns a reference to a cell from a defined range. | reference: reference, row: integer=0, column: integer=0, range: integer=1 | reference | 1..4 | `SC_OPCODE_INDEX` (290) |
| INDIRECT | `INDIRECT(ref; [A1])` | Returns the contents of a cell that is referenced in text form. | ref: text, A1: logical=TRUE (A1) | reference | 1..2 | `SC_OPCODE_INDIRECT` (279) |
| LOOKUP | `LOOKUP(Search_criterion; Search_vector; [result_vector])` | Determines a value in a vector by comparison to values in another vector. | Search criterion: any, Search vector: range, result_vector: range=search vector | any | 2..3 | `SC_OPCODE_LOOKUP` (285) |
| MATCH | `MATCH(Search_criterion; lookup_array; [Type])` | Defines a position in a array after comparing values. | Search criterion: any, lookup_array: range, Type: integer=1 | integer | 2..3 | `SC_OPCODE_MATCH` (281) |
| MULTIPLE.OPERATIONS | `MULTIPLE.OPERATIONS(Formula; Column_cell; Column_replacement; [Row_cell]; [Row_replacement])` | Multiple operations (what-if table): recalculates Formula with input cells replaced by other cells. Created by Data > Multiple Operations; ODF name MULTIPLE.OPERATIONS; UI/English name TABLE in older versions. | Formula: reference, Column cell: reference, Column replacement: reference, Row cell: reference, Row replacement: reference | any | 3..5 | `SC_OPCODE_TABLE_OP` (362) |
| OFFSET | `OFFSET(reference; rows; columns; [height]; [width])` | Returns a reference which has been moved in relation to the starting point. | reference: reference, rows: integer, columns: integer, height: integer=height of reference, width: integer=width of reference | reference | 3..5 | `SC_OPCODE_OFFSET` (289) |
| ROW | `ROW([reference])` | Defines the internal row number of a reference. | reference: reference=formula cell | integer | 0..1 | `SC_OPCODE_ROW` (255) |
| ROWS | `ROWS(array)` | Returns the number of rows in a reference or array. | array: range | integer | 1..1 | `SC_OPCODE_ROWS` (253) |
| SHEET | `SHEET([reference])` | Returns the internal sheet number of a reference or a string. | reference: reference=formula cell sheet | integer | 0..1 | `SC_OPCODE_TABLE` (371) |
| SHEETS | `SHEETS([reference])` | Returns the number of sheets of a given reference. If no parameter has been entered, the total number of sheets in the document is returned. | reference: reference=all sheets in document | integer | 0..1 | `SC_OPCODE_TABLES` (372) |
| STYLE | `STYLE(Style; [Time]; [Style2])` | Applies a Style to the formula cell. | Style: text, Time: number=0, Style2: text=(none) | number | 1..3 | `SC_OPCODE_STYLE` (368) / ODF `ORG.OPENOFFICE.STYLE` |
| VLOOKUP | `VLOOKUP(Search_criterion; array; Index; [sort_order])` | Vertical search and reference to indicated cells. | Search criterion: any, array: range, Index: integer, sort order: logical=TRUE | any | 3..4 | `SC_OPCODE_V_LOOKUP` (286) |

## Text (37)

| Name | Signature | Description | Param types (=default) | Returns | Min..Max | Opcode (id) / ODF name |
|---|---|---|---|---|---|---|
| ARABIC | `ARABIC(Text)` | Calculates the value of a Roman numeral. | Text: text | integer | 1..1 | `SC_OPCODE_ARABIC` (154) |
| ASC | `ASC(text)` | Converts full-width ASCII and katakana characters to half-width. | text: text | text | 1..1 | `SC_OPCODE_ASC` (158) |
| BAHTTEXT | `BAHTTEXT(Number)` | Converts a number to text (Baht). | Number: number | text | 1..1 | `SC_OPCODE_BAHTTEXT` (156) / ODF `COM.MICROSOFT.BAHTTEXT` |
| BASE | `BASE(number; radix; [Minimum_length])` | Converts a positive integer to text from a number system to the base defined. | number: integer, radix: integer, Minimum length: integer=1 | text | 2..3 | `SC_OPCODE_BASE` (370) |
| CHAR | `CHAR(number)` | Converts a code number into a character or letter. | number: integer | text | 1..1 | `SC_OPCODE_CHAR` (142) |
| CLEAN | `CLEAN(text)` | Removes all nonprintable characters from text. | text: text | text | 1..1 | `SC_OPCODE_CLEAN` (141) |
| CODE | `CODE(text)` | Returns a numeric code for the first character in a text string. | text: text | integer | 1..1 | `SC_OPCODE_CODE` (133) |
| CONCATENATE | `CONCATENATE(text1; text2; ...)` | Combines several text items into one. | text: textList | text | 1..255 | `SC_OPCODE_CONCAT` (304) |
| DECIMAL | `DECIMAL(text; radix)` | Converts a text of a specified number system to a positive integer in the base given. | text: text, radix: integer | number | 2..2 | `SC_OPCODE_DECIMAL` (381) |
| DOLLAR | `DOLLAR(value; [decimals])` | Converts a number to text in currency format. | value: number, decimals: integer=2 | text | 1..2 | `SC_OPCODE_CURRENCY` (292) |
| EXACT | `EXACT(text_1; text_2)` | Specifies whether two texts are identical. | text_1: text, text_2: text | logical | 2..2 | `SC_OPCODE_EXACT` (296) |
| FIND | `FIND(find_text; text; [position])` | Looks for a string of text within another (case sensitive) | find_text: text, text: text, position: integer=1 | integer | 2..3 | `SC_OPCODE_FIND` (295) |
| FIXED | `FIXED(number; [Decimals]; [No_thousands_separators])` | Formats a number with a fixed number of places after the decimal point and thousands separator. | number: number, Decimals: integer=2, No thousands separators: logical=FALSE | text | 1..3 | `SC_OPCODE_FIXED` (294) |
| JIS | `JIS(text)` | Converts half-width ASCII and katakana characters to full-width. | text: text | text | 1..1 | `SC_OPCODE_JIS` (157) |
| LEFT | `LEFT(text; [number])` | Returns the first character or characters of a text. | text: text, number: integer=1 | text | 1..2 | `SC_OPCODE_LEFT` (297) |
| LEFTB | `LEFTB(text; [number])` | Returns the first character or characters of a text, with DBCS | text: text, number: integer=1 | text | 1..2 | `SC_OPCODE_LEFTB` (402) |
| LEN | `LEN(text)` | Calculates length of a text string. | text: text | integer | 1..1 | `SC_OPCODE_LEN` (138) |
| LENB | `LENB(text)` | Calculates length of a text string, with DBCS | text: text | integer | 1..1 | `SC_OPCODE_LENB` (400) |
| LOWER | `LOWER(text)` | Converts text to lowercase. | text: text | text | 1..1 | `SC_OPCODE_LOWER` (137) |
| MID | `MID(text; start; number)` | Returns a partial text string of a text. | text: text, start: integer, number: integer | text | 3..3 | `SC_OPCODE_MID` (300) |
| MIDB | `MIDB(text; start; number)` | Returns a partial text string of a text, with DBCS | text: text, start: integer, number: integer | text | 3..3 | `SC_OPCODE_MIDB` (403) |
| NUMBERVALUE | `NUMBERVALUE(text; decimal_point)` | Converts text to a number, in a locale-independent way. | text: text, decimal_point: text | number | 2..2 | `SC_OPCODE_NUMBERVALUE` (392) |
| PROPER | `PROPER(text)` | Capitalizes the first letter in all words. | text: text | text | 1..1 | `SC_OPCODE_PROPPER` (136) |
| REPLACE | `REPLACE(Text; position; length; new_text)` | Replaces characters within a text string with a different text string. | Text: text, position: integer, length: integer, new text: text | text | 4..4 | `SC_OPCODE_REPLACE` (293) |
| REPT | `REPT(text; number)` | Repeats text a given number of times. | text: text, number: integer | text | 2..2 | `SC_OPCODE_REPT` (303) |
| RIGHT | `RIGHT(text; [number])` | Returns the last character or characters of a text. | text: text, number: integer=1 | text | 1..2 | `SC_OPCODE_RIGHT` (298) |
| RIGHTB | `RIGHTB(text; [number])` | Returns the last character or characters of a text, with DBCS | text: text, number: integer=1 | text | 1..2 | `SC_OPCODE_RIGHTB` (401) |
| ROMAN | `ROMAN(Number; [Mode])` | Converts a number to a Roman numeral. | Number: integer, Mode: integer=0 | text | 1..2 | `SC_OPCODE_ROMAN` (383) |
| SEARCH | `SEARCH(find_text; text; [position])` | Looks for one text value within another (not case-sensitive). | find_text: text, text: text, position: integer=1 | integer | 2..3 | `SC_OPCODE_SEARCH` (299) |
| SUBSTITUTE | `SUBSTITUTE(text; search_text; new_text; [occurrence])` | Substitutes new text for old text in a string. | text: text, search_text: text, new text: text, occurrence: integer=(all occurrences) | text | 3..4 | `SC_OPCODE_SUBSTITUTE` (302) |
| T | `T(value)` | Returns a value if it is text, otherwise an empty string. | value: any | text | 1..1 | `SC_OPCODE_T` (139) |
| TEXT | `TEXT(number; Format)` | Converts a number to text according to a given format. | number: number, Format: text | text | 2..2 | `SC_OPCODE_TEXT` (301) |
| TRIM | `TRIM(text)` | Removes extra spaces from text. | text: text | text | 1..1 | `SC_OPCODE_TRIM` (134) |
| UNICHAR | `UNICHAR(number)` | Converts a code number into a Unicode character or letter. | number: integer | text | 1..1 | `SC_OPCODE_UNICHAR` (160) |
| UNICODE | `UNICODE(text)` | Returns the numeric code for the first Unicode character in a text string. | text: text | integer | 1..1 | `SC_OPCODE_UNICODE` (159) |
| UPPER | `UPPER(text)` | Converts text to uppercase. | text: text | text | 1..1 | `SC_OPCODE_UPPER` (135) |
| VALUE | `VALUE(text)` | Converts text to a number. | text: text | number | 1..1 | `SC_OPCODE_VALUE` (140) |

## Add-in (109)

Add-in functions are provided by UNO components. In the Function Wizard they appear in the category the add-in reports (column "Wizard cat."); "Technical" is not a Calc category, so those show under **Add-In**. Names with suffix `_ADD` collide with a built-in of the same base name.

| Name | Wizard cat. | Signature | Description | Param types | Returns | Min..Max | ODF name / UNO programmatic name |
|---|---|---|---|---|---|---|---|
| EDATE | Date&Time | `EDATE(Start_date; Months)` | Returns the serial number of the date that is a specified number of months before or after the start date | Start date: date, Months: integer | date | 2..2 | EDATE / `com.sun.star.sheet.addin.Analysis.getEdate` |
| EOMONTH | Date&Time | `EOMONTH(Start_date; Months)` | Returns the serial number of the last day of the month that comes a certain number of months before or after the start date | Start date: date, Months: integer | date | 2..2 | EOMONTH / `com.sun.star.sheet.addin.Analysis.getEomonth` |
| NETWORKDAYS | Date&Time | `NETWORKDAYS(Start_date; End_date; [Holidays])` | Returns the number of workdays between two dates | Start date: date, End date: date, Holidays: array=(no holidays) | integer | 2..3 | NETWORKDAYS / `com.sun.star.sheet.addin.Analysis.getNetworkdays` |
| WEEKNUM_ADD | Date&Time | `WEEKNUM_ADD(Date; Return_type)` | Returns the number of the calendar week in which the specified date occurs. | Date: date, Return type: integer | integer | 2..2 | WEEKNUM / `com.sun.star.sheet.addin.Analysis.getWeeknum` |
| WORKDAY | Date&Time | `WORKDAY(Start_date; Days; [Holidays])` | Returns the serial number of the date before or after a specified number of workdays | Start date: date, Days: integer, Holidays: array=(no holidays) | date | 2..3 | WORKDAY / `com.sun.star.sheet.addin.Analysis.getWorkday` |
| YEARFRAC | Date&Time | `YEARFRAC(Start_date; End_date; [Basis])` | Returns the number of whole days between 'start date' and 'end date' as a year fraction | Start date: date, End date: date, Basis: integer=0 | number | 2..3 | YEARFRAC / `com.sun.star.sheet.addin.Analysis.getYearfrac` |
| ACCRINT | Financial | `ACCRINT(Issue; First_interest; Settlement; Rate; [Par]; Frequency; [Basis])` | Returns the accrued interest for a security that pays periodic interest | Issue: date, First interest: date, Settlement: date, Rate: number, Par: number=1000, Frequency: integer, Basis: integer=0 (US 30/360) | number | 5..7 | ACCRINT / `com.sun.star.sheet.addin.Analysis.getAccrint` |
| ACCRINTM | Financial | `ACCRINTM(Issue; Settlement; Rate; [Par]; [Basis])` | Returns the accrued interest for a security that pays interest at maturity | Issue: date, Settlement: date, Rate: number, Par: number=1000, Basis: integer=0 (US 30/360) | number | 3..5 | ACCRINTM / `com.sun.star.sheet.addin.Analysis.getAccrintm` |
| AMORDEGRC | Financial | `AMORDEGRC(Cost; Date_purchased; First_period; Salvage; Period; Rate; [Basis])` | Returns the prorated linear depreciation of an asset for each accounting period | Cost: number, Date purchased: date, First period: date, Salvage: number, Period: number, Rate: number, Basis: integer=0 (US 30/360) | number | 6..7 | AMORDEGRC / `com.sun.star.sheet.addin.Analysis.getAmordegrc` |
| AMORLINC | Financial | `AMORLINC(Cost; Date_purchased; First_period; Salvage; Period; Rate; [Basis])` | Returns the prorated linear depreciation of an asset for each accounting period | Cost: number, Date purchased: date, First period: date, Salvage: number, Period: number, Rate: number, Basis: integer=0 (US 30/360) | number | 6..7 | AMORLINC / `com.sun.star.sheet.addin.Analysis.getAmorlinc` |
| COUPDAYBS | Financial | `COUPDAYBS(Settlement; Maturity; Frequency; [Basis])` | Returns the number of days from the beginning of the coupon period to the settlement date | Settlement: date, Maturity: date, Frequency: integer, Basis: integer=0 (US 30/360) | number | 3..4 | COUPDAYBS / `com.sun.star.sheet.addin.Analysis.getCoupdaybs` |
| COUPDAYS | Financial | `COUPDAYS(Settlement; Maturity; Frequency; [Basis])` | Returns the number of days in the coupon period containing the settlement date | Settlement: date, Maturity: date, Frequency: integer, Basis: integer=0 (US 30/360) | number | 3..4 | COUPDAYS / `com.sun.star.sheet.addin.Analysis.getCoupdays` |
| COUPDAYSNC | Financial | `COUPDAYSNC(Settlement; Maturity; Frequency; [Basis])` | Returns the number of days from the settlement date to the next coupon date | Settlement: date, Maturity: date, Frequency: integer, Basis: integer=0 (US 30/360) | number | 3..4 | COUPDAYSNC / `com.sun.star.sheet.addin.Analysis.getCoupdaysnc` |
| COUPNCD | Financial | `COUPNCD(Settlement; Maturity; Frequency; [Basis])` | Returns the first coupon date after the settlement date | Settlement: date, Maturity: date, Frequency: integer, Basis: integer=0 (US 30/360) | date | 3..4 | COUPNCD / `com.sun.star.sheet.addin.Analysis.getCoupncd` |
| COUPNUM | Financial | `COUPNUM(Settlement; Maturity; Frequency; [Basis])` | Returns the number of coupons payable between the settlement and maturity dates | Settlement: date, Maturity: date, Frequency: integer, Basis: integer=0 (US 30/360) | number | 3..4 | COUPNUM / `com.sun.star.sheet.addin.Analysis.getCoupnum` |
| COUPPCD | Financial | `COUPPCD(Settlement; Maturity; Frequency; [Basis])` | Returns the last coupon date preceding the settlement date | Settlement: date, Maturity: date, Frequency: integer, Basis: integer=0 (US 30/360) | date | 3..4 | COUPPCD / `com.sun.star.sheet.addin.Analysis.getCouppcd` |
| CUMIPMT_ADD | Financial | `CUMIPMT_ADD(Rate; Nper; Pv; Start_period; End_period; Type)` | Returns the cumulative interest to be paid between two periods | Rate: number, Nper: integer, Pv: number, Start period: integer, End period: integer, Type: integer | number | 6..6 | CUMIPMT / `com.sun.star.sheet.addin.Analysis.getCumipmt` |
| CUMPRINC_ADD | Financial | `CUMPRINC_ADD(Rate; Nper; Pv; Start_period; End_period; Type)` | Returns the cumulative principal on a loan to be paid between two periods | Rate: number, Nper: integer, Pv: number, Start period: integer, End period: integer, Type: integer | number | 6..6 | CUMPRINC / `com.sun.star.sheet.addin.Analysis.getCumprinc` |
| DISC | Financial | `DISC(Settlement; Maturity; Price; Redemption; [Basis])` | Returns the discount rate for a security | Settlement: date, Maturity: date, Price: number, Redemption: number, Basis: integer=0 (US 30/360) | number | 4..5 | DISC / `com.sun.star.sheet.addin.Analysis.getDisc` |
| DOLLARDE | Financial | `DOLLARDE(Fractional_dollar; Fraction)` | Converts a price expressed as a fraction into a price expressed as a decimal | Fractional dollar: number, Fraction: integer | number | 2..2 | DOLLARDE / `com.sun.star.sheet.addin.Analysis.getDollarde` |
| DOLLARFR | Financial | `DOLLARFR(Decimal_dollar; Fraction)` | Converts a price expressed as a decimal into a price expressed as a fraction | Decimal dollar: number, Fraction: integer | number | 2..2 | DOLLARFR / `com.sun.star.sheet.addin.Analysis.getDollarfr` |
| DURATION_ADD | Financial | `DURATION_ADD(Settlement; Maturity; Coupon; Yield; Frequency; [Basis])` | Returns the annual Macaulay duration of a security with periodic interest payments | Settlement: date, Maturity: date, Coupon: number, Yield: number, Frequency: integer, Basis: integer=0 (US 30/360) | number | 5..6 | DURATION / `com.sun.star.sheet.addin.Analysis.getDuration` |
| EFFECT_ADD | Financial | `EFFECT_ADD(Nominal_rate; Npery)` | Returns the effective annual interest rate | Nominal rate: number, Npery: integer | number | 2..2 | EFFECT / `com.sun.star.sheet.addin.Analysis.getEffect` |
| FVSCHEDULE | Financial | `FVSCHEDULE(Principal; Schedule)` | Returns the future value of the initial principal after a series of compound interest rates are applied | Principal: number, Schedule: array | number | 2..2 | FVSCHEDULE / `com.sun.star.sheet.addin.Analysis.getFvschedule` |
| INTRATE | Financial | `INTRATE(Settlement; Maturity; Investment; Redemption; [Basis])` | Returns the interest rate for a fully invested security | Settlement: date, Maturity: date, Investment: number, Redemption: number, Basis: integer=0 (US 30/360) | number | 4..5 | INTRATE / `com.sun.star.sheet.addin.Analysis.getIntrate` |
| MDURATION | Financial | `MDURATION(Settlement; Maturity; Coupon; Yield; Frequency; [Basis])` | Returns the Macaulay modified duration for a security with an assumed par value of 100 currency units | Settlement: date, Maturity: date, Coupon: number, Yield: number, Frequency: integer, Basis: integer=0 (US 30/360) | number | 5..6 | MDURATION / `com.sun.star.sheet.addin.Analysis.getMduration` |
| NOMINAL_ADD | Financial | `NOMINAL_ADD(Effective_rate; Npery)` | Returns the annual nominal interest rate | Effective rate: number, Npery: integer | number | 2..2 | NOMINAL / `com.sun.star.sheet.addin.Analysis.getNominal` |
| ODDFPRICE | Financial | `ODDFPRICE(Settlement; Maturity; Issue; First_coupon; Rate; Yield; Redemption; Frequency; [Basis])` | Returns the price per $100 face value of a security with an odd first period | Settlement: date, Maturity: date, Issue: date, First coupon: date, Rate: number, Yield: number, Redemption: number, Frequency: integer, Basis: integer=0 (US 30/360) | number | 8..9 | ODDFPRICE / `com.sun.star.sheet.addin.Analysis.getOddfprice` |
| ODDFYIELD | Financial | `ODDFYIELD(Settlement; Maturity; Issue; First_coupon; Rate; Price; Redemption; Frequency; [Basis])` | Returns the yield of a security with an odd first period | Settlement: date, Maturity: date, Issue: date, First coupon: date, Rate: number, Price: number, Redemption: number, Frequency: integer, Basis: integer=0 (US 30/360) | number | 8..9 | ODDFYIELD / `com.sun.star.sheet.addin.Analysis.getOddfyield` |
| ODDLPRICE | Financial | `ODDLPRICE(Settlement; Maturity; Last_interest; Rate; Yield; Redemption; Frequency; [Basis])` | Returns the price per $100 face value of a security with an odd last period | Settlement: date, Maturity: date, Last interest: date, Rate: number, Yield: number, Redemption: number, Frequency: integer, Basis: integer=0 (US 30/360) | number | 7..8 | ODDLPRICE / `com.sun.star.sheet.addin.Analysis.getOddlprice` |
| ODDLYIELD | Financial | `ODDLYIELD(Settlement; Maturity; Last_interest; Rate; Price; Redemption; Frequency; [Basis])` | Returns the yield of a security with an odd last period | Settlement: date, Maturity: date, Last interest: date, Rate: number, Price: number, Redemption: number, Frequency: integer, Basis: integer=0 (US 30/360) | number | 7..8 | ODDLYIELD / `com.sun.star.sheet.addin.Analysis.getOddlyield` |
| PRICE | Financial | `PRICE(Settlement; Maturity; Rate; Yield; Redemption; Frequency; [Basis])` | Returns the price per 100 currency units face value of a security that pays periodic interest | Settlement: date, Maturity: date, Rate: number, Yield: number, Redemption: number, Frequency: integer, Basis: integer=0 (US 30/360) | number | 6..7 | PRICE / `com.sun.star.sheet.addin.Analysis.getPrice` |
| PRICEDISC | Financial | `PRICEDISC(Settlement; Maturity; Discount; Redemption; [Basis])` | Returns the price per 100 currency units face value of a discounted security | Settlement: date, Maturity: date, Discount: number, Redemption: number, Basis: integer=0 (US 30/360) | number | 4..5 | PRICEDISC / `com.sun.star.sheet.addin.Analysis.getPricedisc` |
| PRICEMAT | Financial | `PRICEMAT(Settlement; Maturity; Issue; Rate; Yield; [Basis])` | Returns the price per 100 currency units face value of a security that pays interest at maturity | Settlement: date, Maturity: date, Issue: date, Rate: number, Yield: number, Basis: integer=0 (US 30/360) | number | 5..6 | PRICEMAT / `com.sun.star.sheet.addin.Analysis.getPricemat` |
| RECEIVED | Financial | `RECEIVED(Settlement; Maturity; Investment; Discount; [Basis])` | Returns the amount paid out at maturity for a fully invested security | Settlement: date, Maturity: date, Investment: number, Discount: number, Basis: integer=0 (US 30/360) | number | 4..5 | RECEIVED / `com.sun.star.sheet.addin.Analysis.getReceived` |
| TBILLEQ | Financial | `TBILLEQ(Settlement; Maturity; Discount)` | Returns the bond-equivalent yield for a treasury bill | Settlement: date, Maturity: date, Discount: number | number | 3..3 | TBILLEQ / `com.sun.star.sheet.addin.Analysis.getTbilleq` |
| TBILLPRICE | Financial | `TBILLPRICE(Settlement; Maturity; Discount)` | Returns the price of 100 currency units face value for a treasury bill | Settlement: date, Maturity: date, Discount: number | number | 3..3 | TBILLPRICE / `com.sun.star.sheet.addin.Analysis.getTbillprice` |
| TBILLYIELD | Financial | `TBILLYIELD(Settlement; Maturity; Price)` | Returns the yield for a treasury bill | Settlement: date, Maturity: date, Price: number | number | 3..3 | TBILLYIELD / `com.sun.star.sheet.addin.Analysis.getTbillyield` |
| XIRR | Financial | `XIRR(Values; Dates; [Guess])` | Returns the internal rate of return for a non-periodic schedule of payments | Values: array, Dates: array, Guess: number=0.1 | number | 2..3 | XIRR / `com.sun.star.sheet.addin.Analysis.getXirr` |
| XNPV | Financial | `XNPV(Rate; Values; Dates)` | Returns the net present value for a non-periodic schedule of payments | Rate: number, Values: array, Dates: array | number | 3..3 | XNPV / `com.sun.star.sheet.addin.Analysis.getXnpv` |
| YIELD | Financial | `YIELD(Settlement; Maturity; Rate; Price; Redemption; Frequency; [Basis])` | Returns the yield on a security that pays periodic interest | Settlement: date, Maturity: date, Rate: number, Price: number, Redemption: number, Frequency: integer, Basis: integer=0 (US 30/360) | number | 6..7 | YIELD / `com.sun.star.sheet.addin.Analysis.getYield` |
| YIELDDISC | Financial | `YIELDDISC(Settlement; Maturity; Price; Redemption; [Basis])` | Returns the annual yield for a discounted security | Settlement: date, Maturity: date, Price: number, Redemption: number, Basis: integer=0 (US 30/360) | number | 4..5 | YIELDDISC / `com.sun.star.sheet.addin.Analysis.getYielddisc` |
| YIELDMAT | Financial | `YIELDMAT(Settlement; Maturity; Issue; Rate; Price; [Basis])` | Returns the annual yield of a security that pays interest at maturity | Settlement: date, Maturity: date, Issue: date, Rate: number, Price: number, Basis: integer=0 (US 30/360) | number | 5..6 | YIELDMAT / `com.sun.star.sheet.addin.Analysis.getYieldmat` |
| ISEVEN_ADD | Information | `ISEVEN_ADD(Number)` | Returns the value 'true' if the number is even | Number: integer | logical | 1..1 | ISEVEN / `com.sun.star.sheet.addin.Analysis.getIseven` |
| ISODD_ADD | Information | `ISODD_ADD(Number)` | Returns the value 'true' if the number is odd | Number: integer | logical | 1..1 | ISODD / `com.sun.star.sheet.addin.Analysis.getIsodd` |
| GCD_ADD | Mathematical | `GCD_ADD(Number(s)1; Number(s)2; ...)` | Returns the greatest common divisor | Number(s): numberList | number | 1..255 | GCD / `com.sun.star.sheet.addin.Analysis.getGcd` |
| LCM_ADD | Mathematical | `LCM_ADD(Number(s)1; Number(s)2; ...)` | Returns the least common multiple | Number(s): numberList | number | 1..255 | LCM / `com.sun.star.sheet.addin.Analysis.getLcm` |
| MROUND | Mathematical | `MROUND(Number; Multiple)` | Returns a number rounded to a specified multiple | Number: number, Multiple: number | number | 2..2 | MROUND / `com.sun.star.sheet.addin.Analysis.getMround` |
| MULTINOMIAL | Mathematical | `MULTINOMIAL(Number(s)1; Number(s)2; ...)` | Returns the multinomial coefficient of a set of numbers | Number(s): numberList | number | 1..255 | MULTINOMIAL / `com.sun.star.sheet.addin.Analysis.getMultinomial` |
| QUOTIENT | Mathematical | `QUOTIENT(Numerator; Denominator)` | Returns the integer portion of a division | Numerator: number, Denominator: number | number | 2..2 | QUOTIENT / `com.sun.star.sheet.addin.Analysis.getQuotient` |
| RANDBETWEEN | Mathematical | `RANDBETWEEN(Bottom; Top)` | Returns a random integer between the numbers you specify | Bottom: number, Top: number | number | 2..2 | RANDBETWEEN / `com.sun.star.sheet.addin.Analysis.getRandbetween` |
| SERIESSUM | Mathematical | `SERIESSUM(X; N; M; Coefficients)` | Returns the sum of a power series | X: number, N: number, M: number, Coefficients: array | number | 4..4 | SERIESSUM / `com.sun.star.sheet.addin.Analysis.getSeriessum` |
| SQRTPI | Mathematical | `SQRTPI(Number)` | Returns the square root of a number which has been multiplied by pi | Number: number | number | 1..1 | SQRTPI / `com.sun.star.sheet.addin.Analysis.getSqrtpi` |
| BESSELI | Technical | `BESSELI(X; N)` | Returns the modified Bessel function In(x) | X: number, N: integer | number | 2..2 | BESSELI / `com.sun.star.sheet.addin.Analysis.getBesseli` |
| BESSELJ | Technical | `BESSELJ(X; N)` | Returns the Bessel function Jn(x) | X: number, N: integer | number | 2..2 | BESSELJ / `com.sun.star.sheet.addin.Analysis.getBesselj` |
| BESSELK | Technical | `BESSELK(X; N)` | Returns the Bessel function Kn(x) | X: number, N: integer | number | 2..2 | BESSELK / `com.sun.star.sheet.addin.Analysis.getBesselk` |
| BESSELY | Technical | `BESSELY(X; N)` | Returns the Bessel function Yn(x) | X: number, N: integer | number | 2..2 | BESSELY / `com.sun.star.sheet.addin.Analysis.getBessely` |
| BIN2DEC | Technical | `BIN2DEC(Number)` | Converts a binary number to a decimal number | Number: text | number | 1..1 | BIN2DEC / `com.sun.star.sheet.addin.Analysis.getBin2Dec` |
| BIN2HEX | Technical | `BIN2HEX(Number; [Places])` | Converts a binary number to a hexadecimal number | Number: text, Places: integer=(minimum needed) | text | 1..2 | BIN2HEX / `com.sun.star.sheet.addin.Analysis.getBin2Hex` |
| BIN2OCT | Technical | `BIN2OCT(Number; [Places])` | Converts a binary number to an octal number | Number: text, Places: integer=(minimum needed) | text | 1..2 | BIN2OCT / `com.sun.star.sheet.addin.Analysis.getBin2Oct` |
| COMPLEX | Technical | `COMPLEX(Real_num; I_num; [Suffix])` | Converts real and imaginary coefficients into a complex number | Real num: number, I num: number, Suffix: text="i" | text | 2..3 | COMPLEX / `com.sun.star.sheet.addin.Analysis.getComplex` |
| CONVERT_ADD | Technical | `CONVERT_ADD(Number; From_unit; To_unit)` | Converts a number from one measurement system to another | Number: number, From unit: text, To unit: text | number | 3..3 | CONVERT / `com.sun.star.sheet.addin.Analysis.getConvert` |
| DEC2BIN | Technical | `DEC2BIN(Number; [Places])` | Converts a decimal number to a binary number | Number: integer, Places: integer=(minimum needed) | text | 1..2 | DEC2BIN / `com.sun.star.sheet.addin.Analysis.getDec2Bin` |
| DEC2HEX | Technical | `DEC2HEX(Number; [Places])` | Converts a decimal number to a hexadecimal number | Number: number, Places: integer=(minimum needed) | text | 1..2 | DEC2HEX / `com.sun.star.sheet.addin.Analysis.getDec2Hex` |
| DEC2OCT | Technical | `DEC2OCT(Number; [Places])` | Converts a decimal number into an octal number | Number: integer, Places: integer=(minimum needed) | text | 1..2 | DEC2OCT / `com.sun.star.sheet.addin.Analysis.getDec2Oct` |
| DELTA | Technical | `DELTA(Number_1; [Number_2])` | Tests whether two values are equal | Number 1: number, Number 2: number=0 | integer | 1..2 | DELTA / `com.sun.star.sheet.addin.Analysis.getDelta` |
| ERF | Technical | `ERF(Lower_limit; [Upper_limit])` | Returns the error function | Lower limit: number, Upper limit: number=(omitted: result is ERF over 0..Lower limit) | number | 1..2 | ERF / `com.sun.star.sheet.addin.Analysis.getErf` |
| ERFC | Technical | `ERFC(Lower_limit)` | Returns the complementary error function | Lower limit: number | number | 1..1 | ERFC / `com.sun.star.sheet.addin.Analysis.getErfc` |
| FACTDOUBLE | Technical | `FACTDOUBLE(Number)` | Returns the double factorial of Number | Number: integer | number | 1..1 | FACTDOUBLE / `com.sun.star.sheet.addin.Analysis.getFactdouble` |
| GESTEP | Technical | `GESTEP(Number; [Step])` | Tests whether a number is greater than a threshold value | Number: number, Step: number=0 | integer | 1..2 | GESTEP / `com.sun.star.sheet.addin.Analysis.getGestep` |
| HEX2BIN | Technical | `HEX2BIN(Number; [Places])` | Converts a hexadecimal number to a binary number | Number: text, Places: integer=(minimum needed) | text | 1..2 | HEX2BIN / `com.sun.star.sheet.addin.Analysis.getHex2Bin` |
| HEX2DEC | Technical | `HEX2DEC(Number)` | Converts a hexadecimal number to a decimal number | Number: text | number | 1..1 | HEX2DEC / `com.sun.star.sheet.addin.Analysis.getHex2Dec` |
| HEX2OCT | Technical | `HEX2OCT(Number; [Places])` | Converts a hexadecimal number to an octal number | Number: text, Places: integer=(minimum needed) | text | 1..2 | HEX2OCT / `com.sun.star.sheet.addin.Analysis.getHex2Oct` |
| IMABS | Technical | `IMABS(Complex_number)` | Returns the absolute value (modulus) of a complex number | Complex number: text | number | 1..1 | IMABS / `com.sun.star.sheet.addin.Analysis.getImabs` |
| IMAGINARY | Technical | `IMAGINARY(Complex_number)` | Returns the imaginary coefficient of a complex number | Complex number: text | number | 1..1 | IMAGINARY / `com.sun.star.sheet.addin.Analysis.getImaginary` |
| IMARGUMENT | Technical | `IMARGUMENT(Complex_number)` | Returns the argument theta, an angle expressed in radians | Complex number: text | number | 1..1 | IMARGUMENT / `com.sun.star.sheet.addin.Analysis.getImargument` |
| IMCONJUGATE | Technical | `IMCONJUGATE(Complex_number)` | Returns the complex conjugate of a complex number | Complex number: text | text | 1..1 | IMCONJUGATE / `com.sun.star.sheet.addin.Analysis.getImconjugate` |
| IMCOS | Technical | `IMCOS(Complex_number)` | Returns the cosine of a complex number | Complex number: text | text | 1..1 | IMCOS / `com.sun.star.sheet.addin.Analysis.getImcos` |
| IMCOSH | Technical | `IMCOSH(Complex_number)` | Returns the hyperbolic cosine of a complex number | Complex number: text | text | 1..1 | IMCOSH / `com.sun.star.sheet.addin.Analysis.getImcosh` |
| IMCOT | Technical | `IMCOT(Complex_number)` | Returns the cotangent of a complex number | Complex number: text | text | 1..1 | IMCOT / `com.sun.star.sheet.addin.Analysis.getImcot` |
| IMCSC | Technical | `IMCSC(Complex_number)` | Returns the cosecant of a complex number | Complex number: text | text | 1..1 | IMCSC / `com.sun.star.sheet.addin.Analysis.getImcsc` |
| IMCSCH | Technical | `IMCSCH(Complex_number)` | Returns the hyperbolic cosecant of a complex number | Complex number: text | text | 1..1 | IMCSCH / `com.sun.star.sheet.addin.Analysis.getImcsch` |
| IMDIV | Technical | `IMDIV(Numerator; Denominator)` | Returns the quotient of two complex numbers | Numerator: text, Denominator: text | text | 2..2 | IMDIV / `com.sun.star.sheet.addin.Analysis.getImdiv` |
| IMEXP | Technical | `IMEXP(Complex_number)` | Returns the algebraic form of the exponential of a complex number | Complex number: text | text | 1..1 | IMEXP / `com.sun.star.sheet.addin.Analysis.getImexp` |
| IMLN | Technical | `IMLN(Complex_number)` | Returns the natural logarithm of a complex number | Complex number: text | text | 1..1 | IMLN / `com.sun.star.sheet.addin.Analysis.getImln` |
| IMLOG10 | Technical | `IMLOG10(Complex_number)` | Returns the base-10 logarithm of a complex number | Complex number: text | text | 1..1 | IMLOG10 / `com.sun.star.sheet.addin.Analysis.getImlog10` |
| IMLOG2 | Technical | `IMLOG2(Complex_number)` | Returns the base-2 logarithm of a complex number | Complex number: text | text | 1..1 | IMLOG2 / `com.sun.star.sheet.addin.Analysis.getImlog2` |
| IMPOWER | Technical | `IMPOWER(Complex_number; Number)` | Returns a complex number raised to an integer power | Complex number: text, Number: number | text | 2..2 | IMPOWER / `com.sun.star.sheet.addin.Analysis.getImpower` |
| IMPRODUCT | Technical | `IMPRODUCT(Complex_number; [Complex_number1; Complex_number2; ...])` | Returns the product of several complex numbers | Complex number: range, Complex number: textList | text | 1..255 | IMPRODUCT / `com.sun.star.sheet.addin.Analysis.getImproduct` |
| IMREAL | Technical | `IMREAL(Complex_number)` | Returns the real coefficient of a complex number | Complex number: text | number | 1..1 | IMREAL / `com.sun.star.sheet.addin.Analysis.getImreal` |
| IMSEC | Technical | `IMSEC(Complex_number)` | Returns the secant of a complex number | Complex number: text | text | 1..1 | IMSEC / `com.sun.star.sheet.addin.Analysis.getImsec` |
| IMSECH | Technical | `IMSECH(Complex_number)` | Returns the hyperbolic secant of a complex number | Complex number: text | text | 1..1 | IMSECH / `com.sun.star.sheet.addin.Analysis.getImsech` |
| IMSIN | Technical | `IMSIN(Complex_number)` | Returns the sine of a complex number | Complex number: text | text | 1..1 | IMSIN / `com.sun.star.sheet.addin.Analysis.getImsin` |
| IMSINH | Technical | `IMSINH(Complex_number)` | Returns the hyperbolic sine of a complex number | Complex number: text | text | 1..1 | IMSINH / `com.sun.star.sheet.addin.Analysis.getImsinh` |
| IMSQRT | Technical | `IMSQRT(Complex_number)` | Returns the square root of a complex number | Complex number: text | text | 1..1 | IMSQRT / `com.sun.star.sheet.addin.Analysis.getImsqrt` |
| IMSUB | Technical | `IMSUB(Complex_number_1; Complex_number_2)` | Returns the difference of two complex numbers | Complex number 1: text, Complex number 2: text | text | 2..2 | IMSUB / `com.sun.star.sheet.addin.Analysis.getImsub` |
| IMSUM | Technical | `IMSUM(Complex_number1; Complex_number2; ...)` | Returns the sum of complex numbers | Complex number: textList | text | 1..255 | IMSUM / `com.sun.star.sheet.addin.Analysis.getImsum` |
| IMTAN | Technical | `IMTAN(Complex_number)` | Returns the tangent of a complex number | Complex number: text | text | 1..1 | IMTAN / `com.sun.star.sheet.addin.Analysis.getImtan` |
| OCT2BIN | Technical | `OCT2BIN(Number; [Places])` | Converts an octal number to a binary number | Number: text, Places: integer=(minimum needed) | text | 1..2 | OCT2BIN / `com.sun.star.sheet.addin.Analysis.getOct2Bin` |
| OCT2DEC | Technical | `OCT2DEC(Number)` | Converts an octal number to a decimal number | Number: text | number | 1..1 | OCT2DEC / `com.sun.star.sheet.addin.Analysis.getOct2Dec` |
| OCT2HEX | Technical | `OCT2HEX(Number; [Places])` | Converts an octal number to a hexadecimal number | Number: text, Places: integer=(minimum needed) | text | 1..2 | OCT2HEX / `com.sun.star.sheet.addin.Analysis.getOct2Hex` |
| DAYSINMONTH | Date&Time | `DAYSINMONTH(Date)` | Returns the number of days of the month in which the date entered occurs | Date: date | integer | 1..1 | ORG.OPENOFFICE.DAYSINMONTH / `com.sun.star.sheet.addin.DateFunctions.getDaysInMonth` |
| DAYSINYEAR | Date&Time | `DAYSINYEAR(Date)` | Returns the number of days of the year in which the date entered occurs. | Date: date | integer | 1..1 | ORG.OPENOFFICE.DAYSINYEAR / `com.sun.star.sheet.addin.DateFunctions.getDaysInYear` |
| ISLEAPYEAR | Date&Time | `ISLEAPYEAR(Date)` | Returns 1 (TRUE) if the date is a day of a leap year, otherwise 0 (FALSE). | Date: date | logical | 1..1 | ORG.OPENOFFICE.ISLEAPYEAR / `com.sun.star.sheet.addin.DateFunctions.getIsLeapYear` |
| MONTHS | Date&Time | `MONTHS(Start_date; End_date; Type)` | Determines the number of months in a specific period. | Start date: date, End date: date, Type: integer | integer | 3..3 | ORG.OPENOFFICE.MONTHS / `com.sun.star.sheet.addin.DateFunctions.getDiffMonths` |
| WEEKS | Date&Time | `WEEKS(Start_date; End_date; Type)` | Calculates the number of weeks in a specific period | Start date: date, End date: date, Type: integer | integer | 3..3 | ORG.OPENOFFICE.WEEKS / `com.sun.star.sheet.addin.DateFunctions.getDiffWeeks` |
| WEEKSINYEAR | Date&Time | `WEEKSINYEAR(Date)` | Returns the number of weeks of the year in which the date entered occurs | Date: date | integer | 1..1 | ORG.OPENOFFICE.WEEKSINYEAR / `com.sun.star.sheet.addin.DateFunctions.getWeeksInYear` |
| YEARS | Date&Time | `YEARS(Start_date; End_date; Type)` | Calculates the number of years in a specific period. | Start date: date, End date: date, Type: integer | integer | 3..3 | ORG.OPENOFFICE.YEARS / `com.sun.star.sheet.addin.DateFunctions.getDiffYears` |
| ROT13 | Text | `ROT13(Text)` | Encrypts or decrypts a text using the ROT13 algorithm | Text: text | text | 1..1 | ORG.OPENOFFICE.ROT13 / `com.sun.star.sheet.addin.DateFunctions.getRot13` |

## Function-specific notes from source

- **WEEKNUM**: Built-in WEEKNUM (ODF ISOWEEKNUM): mode 1 = week starts Sunday, otherwise ISO 8601 (Monday, first week contains Jan 4). The Excel-compatible variant is WEEKNUM_ADD (Analysis add-in).
- **CELL**: Returns: text or number depending on info_type
- **COLUMN**: Returns: integer, or array of column numbers when called with a multi-column range in array context
- **ROW**: Returns: integer, or array of row numbers when called with a multi-row range in array context
- **INDEX**: Returns: reference (or value when used on an inline array)
- **HYPERLINK**: Returns: CellText (or URL) value; cell becomes a clickable link
- **GETPIVOTDATA**: Variadic tail is alternating Field name / Item pairs (ODF: GETPIVOTDATA(DataField; PivotTable[; Field1; Item1[; ...]])) or a single constraint string.
- **FIXED**: Function Wizard marks Decimals as required, but interpreter accepts 1..3 args (default Decimals=2).
- **CONVERT**: Built-in legacy CONVERT (ODF name ORG.OPENOFFICE.CONVERT) uses the currency/unit table from configuration Office.Calc/UnitConversion (Euro legacy currencies). The Excel-compatible CONVERT is the Analysis add-in CONVERT_ADD.
- **NUMBERVALUE**: Suppressed in scfuncs.src (flag=1, "TODO: implementation"); no interpreter implementation in ScInterpreter::Interpret - evaluates to an error in AOO.
- **MULTIPLE.OPERATIONS**: No Function Wizard entry in scfuncs.src; English name list core_resource.src: "MULTIPLE.OPERATIONS" (ODFF/UI) and "TABLE" (legacy English API).
- **WEEKNUM_ADD**: Name collides with built-in WEEKNUM (whose ODF name differs), so the add-in is displayed as WEEKNUM_ADD. odffmap.cxx bMapDupToInternal=false: the ODF/Excel name WEEKNUM refers to THIS add-in (Excel-compatible semantics).
- **ISEVEN_ADD**: Name collides with built-in ISEVEN, so the add-in is displayed as ISEVEN_ADD. odffmap.cxx bMapDupToInternal=true: the ODF name ISEVEN is mapped to the internal built-in on import; the add-in variant is only reachable by its _ADD name / programmatic name. Returns: IDL returns long 0/1
- **ISODD_ADD**: Name collides with built-in ISODD, so the add-in is displayed as ISODD_ADD. odffmap.cxx bMapDupToInternal=true: the ODF name ISODD is mapped to the internal built-in on import; the add-in variant is only reachable by its _ADD name / programmatic name. Returns: IDL returns long 0/1
- **GCD_ADD**: Name collides with built-in GCD, so the add-in is displayed as GCD_ADD. odffmap.cxx bMapDupToInternal=true: the ODF name GCD is mapped to the internal built-in on import; the add-in variant is only reachable by its _ADD name / programmatic name.
- **LCM_ADD**: Name collides with built-in LCM, so the add-in is displayed as LCM_ADD. odffmap.cxx bMapDupToInternal=true: the ODF name LCM is mapped to the internal built-in on import; the add-in variant is only reachable by its _ADD name / programmatic name.
- **IMPOWER**: Returns: complex number as text "x+yi" / "x+yj"
- **IMCOS**: Returns: complex number as text "x+yi" / "x+yj"
- **IMDIV**: Returns: complex number as text "x+yi" / "x+yj"
- **IMEXP**: Returns: complex number as text "x+yi" / "x+yj"
- **IMCONJUGATE**: Returns: complex number as text "x+yi" / "x+yj"
- **IMLN**: Returns: complex number as text "x+yi" / "x+yj"
- **IMLOG10**: Returns: complex number as text "x+yi" / "x+yj"
- **IMLOG2**: Returns: complex number as text "x+yi" / "x+yj"
- **IMPRODUCT**: Returns: complex number as text "x+yi" / "x+yj"
- **IMSIN**: Returns: complex number as text "x+yi" / "x+yj"
- **IMSUB**: Returns: complex number as text "x+yi" / "x+yj"
- **IMSQRT**: Returns: complex number as text "x+yi" / "x+yj"
- **IMSUM**: Returns: complex number as text "x+yi" / "x+yj"
- **IMTAN**: Returns: complex number as text "x+yi" / "x+yj"
- **IMSEC**: Returns: complex number as text "x+yi" / "x+yj"
- **IMCSC**: Returns: complex number as text "x+yi" / "x+yj"
- **IMCOT**: Returns: complex number as text "x+yi" / "x+yj"
- **IMSINH**: Returns: complex number as text "x+yi" / "x+yj"
- **IMCOSH**: Returns: complex number as text "x+yi" / "x+yj"
- **IMSECH**: Returns: complex number as text "x+yi" / "x+yj"
- **IMCSCH**: Returns: complex number as text "x+yi" / "x+yj"
- **COMPLEX**: Suffix must be "i" or "j". Returns: complex number as text "x+yi" / "x+yj"
- **CONVERT_ADD**: Name collides with built-in CONVERT (whose ODF name differs), so the add-in is displayed as CONVERT_ADD. odffmap.cxx bMapDupToInternal=false: the ODF/Excel name CONVERT refers to THIS add-in (Excel-compatible semantics).
- **DURATION_ADD**: Name collides with built-in DURATION (whose ODF name differs), so the add-in is displayed as DURATION_ADD. odffmap.cxx bMapDupToInternal=false: the ODF/Excel name DURATION refers to THIS add-in (Excel-compatible semantics).
- **EFFECT_ADD**: Name collides with built-in EFFECT, so the add-in is displayed as EFFECT_ADD. odffmap.cxx bMapDupToInternal=true: the ODF name EFFECT is mapped to the internal built-in on import; the add-in variant is only reachable by its _ADD name / programmatic name.
- **CUMPRINC_ADD**: Name collides with built-in CUMPRINC, so the add-in is displayed as CUMPRINC_ADD. odffmap.cxx bMapDupToInternal=true: the ODF name CUMPRINC is mapped to the internal built-in on import; the add-in variant is only reachable by its _ADD name / programmatic name.
- **CUMIPMT_ADD**: Name collides with built-in CUMIPMT, so the add-in is displayed as CUMIPMT_ADD. odffmap.cxx bMapDupToInternal=true: the ODF name CUMIPMT is mapped to the internal built-in on import; the add-in variant is only reachable by its _ADD name / programmatic name.
- **NOMINAL_ADD**: Name collides with built-in NOMINAL, so the add-in is displayed as NOMINAL_ADD. odffmap.cxx bMapDupToInternal=true: the ODF name NOMINAL is mapped to the internal built-in on import; the add-in variant is only reachable by its _ADD name / programmatic name.
- **ISLEAPYEAR**: Returns: IDL returns long 0/1
