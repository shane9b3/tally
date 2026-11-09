/**
 * The list box next to a range field that offers flagged names (AOO's
 * Advanced Filter and Print Ranges dialogs): picking a name fills the field,
 * typing selects the matching name, else "User defined" / "None".
 */
import { t } from "../i18n";
import { matchChoice, type NameChoice } from "../nameUsage";

/** Value of the "user defined" entry (not a valid name, so it cannot clash). */
const USER = " user";

export function NameChoiceSelect({
  choices,
  text,
  onPick,
  label,
  className,
  testId,
}: {
  choices: readonly NameChoice[];
  text: string;
  onPick: (text: string) => void;
  label: string;
  className?: string;
  testId?: string;
}) {
  if (choices.length === 0) return null;
  const hit = matchChoice(choices, text);
  const user = hit === null && text.trim() !== "";
  return (
    <select
      className={className}
      aria-label={label}
      value={hit ? hit.name : user ? USER : ""}
      onChange={(e) => {
        if (e.target.value === USER) return;
        const c = choices.find((x) => x.name === e.target.value);
        onPick(c ? c.text : "");
      }}
      data-testid={testId}
    >
      <option value="">{t("names.choiceNone", "— none —")}</option>
      {user ? <option value={USER}>{t("names.choiceUser", "— user defined —")}</option> : null}
      {choices.map((c) => (
        <option key={c.name} value={c.name}>
          {c.name}
        </option>
      ))}
    </select>
  );
}
