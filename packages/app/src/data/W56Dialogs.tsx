/** The W5-6 dialog dispatcher, in its own module so the dialogs load as one lazy chunk. */
import type { W56Dialog } from "./store";
import { CustomizeToolbarsDialog } from "../chrome/ChromeUi";
import { AutoFormatDialog, AutoFormatNameDialog, FormatCellsDialog, StyleEditorDialog } from "../styles/StylesUi";
import { OptionsDialog } from "../options/OptionsDialog";
import { ZoomDialog } from "../objects/ZoomDialog";
import { DataPilotDialog } from "../pivot/DataPilotDialog";
import {
  ConsolidateDialog,
  CopySheetDialog,
  CreateNamesDialog,
  DefineDatabaseRangeDialog,
  DeleteCellsDialog,
  DeleteContentsDialog,
  DocumentStatsDialog,
  FillSeriesDialog,
  InsertCellsDialog,
  MergePromptDialog,
  MultipleOperationsDialog,
  RecoveryDialog,
  RemoveDuplicatesDialog,
  SaveAsTemplateDialog,
  ScenariosDialog,
  SelectionListPopover,
  ShowOutlineLevelDialog,
  ShowSheetDialog,
  SolverDialog,
  SortOptionsDialog,
  StandardFilterDialog,
  SubtotalsDialog,
  SelectDatabaseRangeDialog,
  TabColorDialog,
  TextToColumnsDialog,
} from "./DataDialogs";

export function W56DialogFor({ d }: { d: W56Dialog }) {
  switch (d.kind) {
    case "dataPilot":
      return <DataPilotDialog d={d} />;
    case "standardFilter":
      return <StandardFilterDialog d={d} />;
    case "subtotals":
      return <SubtotalsDialog d={d} />;
    case "consolidate":
      return <ConsolidateDialog />;
    case "textToColumns":
      return <TextToColumnsDialog d={d} />;
    case "removeDuplicates":
      return <RemoveDuplicatesDialog d={d} />;
    case "scenarios":
      return <ScenariosDialog d={d} />;
    case "defineDatabaseRange":
      return <DefineDatabaseRangeDialog d={d} />;
    case "selectDatabaseRange":
      return <SelectDatabaseRangeDialog d={d} />;
    case "createNamesFromLabels":
      return <CreateNamesDialog d={d} />;
    case "fillSeries":
      return <FillSeriesDialog d={d} />;
    case "sortOptions":
      return <SortOptionsDialog d={d} />;
    case "deleteContents":
      return <DeleteContentsDialog d={d} />;
    case "multipleOperations":
      return <MultipleOperationsDialog d={d} />;
    case "insertCells":
      return <InsertCellsDialog d={d} />;
    case "deleteCells":
      return <DeleteCellsDialog d={d} />;
    case "copySheet":
      return <CopySheetDialog d={d} />;
    case "showSheet":
      return <ShowSheetDialog d={d} />;
    case "tabColor":
      return <TabColorDialog d={d} />;
    case "showOutlineLevel":
      return <ShowOutlineLevelDialog d={d} />;
    case "solver":
      return <SolverDialog d={d} />;
    case "mergePrompt":
      return <MergePromptDialog d={d} />;
    case "autoFormat":
      return <AutoFormatDialog key={d.select ?? ""} d={d} />;
    case "autoFormatName":
      return <AutoFormatNameDialog d={d} />;
    case "formatCells":
      return <FormatCellsDialog d={d} />;
    case "styleEditor":
      return <StyleEditorDialog d={d} />;
    case "options":
      return <OptionsDialog d={d} />;
    case "documentStats":
      return <DocumentStatsDialog d={d} />;
    case "recovery":
      return <RecoveryDialog d={d} />;
    case "saveAsTemplate":
      return <SaveAsTemplateDialog d={d} />;
    case "selectionList":
      return <SelectionListPopover d={d} />;
    case "customizeToolbars":
      return <CustomizeToolbarsDialog />;
    case "zoom":
      return <ZoomDialog d={d} />;
  }
}
