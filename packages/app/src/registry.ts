/**
 * CommandRegistry: every toolbar, menu, keymap and palette action. Menu
 * `menu-action` ids and assist `Suggestion.action` ids are command ids.
 */

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
  run: () => void | Promise<void>;
}

export class CommandRegistry {
  private readonly commands = new Map<string, Command>();

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
    return c !== undefined && (c.enabled?.() ?? true);
  }

  /** Visible commands in registration order. */
  list(): Command[] {
    return [...this.commands.values()].filter((c) => !c.hidden);
  }

  /** Runs a command; returns false if unknown or disabled. */
  run(id: string): boolean {
    const c = this.commands.get(id);
    if (!c || !(c.enabled?.() ?? true)) return false;
    void c.run();
    return true;
  }
}
