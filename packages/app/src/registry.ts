/**
 * CommandRegistry: every toolbar, menu, keymap and palette action. Menu
 * `menu-action` ids and assist `Suggestion.action` ids are command ids.
 */

/** Named arguments for a command run (from an assist suggestion's `actionArgs`). */
export type CommandArgs = Readonly<Record<string, string>>;

export interface Command {
  id: string;
  title: string;
  /** Extra words the palette matches on. */
  keywords?: string[];
  /** Shortcut label, e.g. "⌘B". */
  shortcut?: string;
  /** Grey label in the palette (Edit, Format, Sheet, File, View, Filter, Chart…). */
  kind: string;
  /** Hidden from the palette (still runnable). */
  hidden?: boolean;
  /** Disabled commands show but do nothing; `reason` explains. */
  enabled?: () => boolean;
  reason?: string;
  /** `args` is set when an assist suggestion runs the command. */
  run: (args?: CommandArgs) => void | Promise<void>;
}

/**
 * A state that disables every command except `allow(id)` (a formula edit
 * picking references on another sheet: AOO disables the slots during
 * reference input; a read-only document: AOO disables the editing slots),
 * with the reason shown for them. Several gates may be installed; the
 * first active one that refuses a command gives its reason.
 */
export interface CommandGate {
  active(): boolean;
  allow(id: string): boolean;
  reason(): string;
}

export class CommandRegistry {
  private readonly commands = new Map<string, Command>();
  private gates: CommandGate[] = [];

  /** Installs `gate` as the only gate (`null` removes every gate). */
  setGate(gate: CommandGate | null): void {
    this.gates = gate ? [gate] : [];
  }

  /** Adds a gate checked by `isEnabled` and `run`; returns its removal. */
  addGate(gate: CommandGate): () => void {
    this.gates = [...this.gates, gate];
    return () => {
      this.gates = this.gates.filter((g) => g !== gate);
    };
  }

  /** The first active gate that refuses `id`. */
  private refusing(id: string): CommandGate | undefined {
    return this.gates.find((g) => !g.allow(id) && g.active());
  }

  private gated(id: string): boolean {
    return this.refusing(id) !== undefined;
  }

  /** Why a command is disabled now (a gate's reason first), if it says. */
  reasonFor(id: string): string | undefined {
    const g = this.refusing(id);
    if (g) return g.reason();
    return this.commands.get(id)?.reason;
  }

  register(cmd: Command): () => void {
    this.commands.set(cmd.id, cmd);
    return () => {
      if (this.commands.get(cmd.id) === cmd) this.commands.delete(cmd.id);
    };
  }

  registerAll(cmds: Command[]): () => void {
    const offs = cmds.map((c) => this.register(c));
    return () => offs.forEach((f) => f());
  }

  get(id: string): Command | undefined {
    return this.commands.get(id);
  }

  has(id: string): boolean {
    return this.commands.has(id);
  }

  isEnabled(id: string): boolean {
    const c = this.commands.get(id);
    return c !== undefined && !this.gated(id) && (c.enabled?.() ?? true);
  }

  /** Visible commands in registration order. */
  list(): Command[] {
    return [...this.commands.values()].filter((c) => !c.hidden);
  }

  /** Runs a command (with optional arguments); returns false if unknown or disabled. */
  run(id: string, args?: CommandArgs): boolean {
    const c = this.commands.get(id);
    if (!c || this.gated(id) || !(c.enabled?.() ?? true)) return false;
    void c.run(args);
    return true;
  }
}
