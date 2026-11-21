import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  applyImportedCommand,
  formatCommandValue,
  isApplyable,
  type ImportedCommands,
} from "@/modules/settings/importedCommands";
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { toast } from "sonner";

type Decision = {
  applied: string[];
  discarded: string[];
};

/**
 * Reviews the command-bearing settings a legacy Terex/Terax import held back.
 * Nothing is active until the user checks it and applies: the default is an
 * empty selection, and dismissing the dialog leaves the quarantine intact so
 * the prompt returns on the next launch.
 */
export function ImportedCommandsReview() {
  const [pending, setPending] = useState<ImportedCommands | null>(null);
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void (async () => {
      try {
        const result = await invoke<ImportedCommands>("imported_commands_read");
        if (result.commands.length > 0) setPending(result);
      } catch {
        // Fail closed: no prompt, nothing applied.
      }
    })();
  }, []);

  const commands = pending?.commands ?? [];

  const toggle = (key: string, next: boolean) => {
    setSelected((current) => {
      const updated = new Set(current);
      if (next) updated.add(key);
      else updated.delete(key);
      return updated;
    });
  };

  const decide = async (decision: Decision) => {
    if (busy) return;
    setBusy(true);
    try {
      for (const command of commands) {
        if (!decision.applied.includes(command.key)) continue;
        if (!isApplyable(command.key)) continue;
        await applyImportedCommand(command.key, command.value);
      }
      await invoke("imported_commands_resolve", {
        applied: decision.applied,
        discarded: decision.discarded,
      });
      const applied = decision.applied.length;
      setPending(null);
      setSelected(new Set());
      if (applied > 0) {
        toast.success(
          applied === 1
            ? "Applied 1 imported command."
            : `Applied ${applied} imported commands.`,
        );
      } else {
        toast.info("Discarded the imported commands.");
      }
    } catch (error) {
      toast.error(`Could not update imported commands: ${String(error)}`);
    } finally {
      setBusy(false);
    }
  };

  const applySelected = () => {
    const applied = commands
      .filter((command) => selected.has(command.key))
      .map((command) => command.key);
    const discarded = commands
      .filter((command) => !selected.has(command.key))
      .map((command) => command.key);
    void decide({ applied, discarded });
  };

  const discardAll = () => {
    void decide({
      applied: [],
      discarded: commands.map((command) => command.key),
    });
  };

  if (!pending) return null;

  return (
    <Dialog
      open
      onOpenChange={(value) => {
        // Dismissing defers the decision; the quarantine is left untouched so
        // the prompt comes back next launch.
        if (!value) {
          setPending(null);
          setSelected(new Set());
        }
      }}
    >
      <DialogContent showCloseButton={!busy}>
        <DialogHeader>
          <DialogTitle>Review imported commands</DialogTitle>
          <DialogDescription>
            {`Tarex imported settings from ${pending.source || "a previous installation"}, but held back the parts that can run programs: your shell, formatter, agent launch commands, and custom language servers. Nothing below runs until you apply it.`}
          </DialogDescription>
        </DialogHeader>

        <ScrollArea className="max-h-[45vh] pr-3">
          <ul className="space-y-3">
            {commands.map((command) => {
              const checked = selected.has(command.key);
              return (
                <li
                  key={command.key}
                  className="rounded-xl border border-border/70 bg-muted/40 p-3"
                >
                  <div className="flex items-center gap-2">
                    <Checkbox
                      id={`imported-${command.key}`}
                      checked={checked}
                      disabled={busy}
                      onCheckedChange={(value) =>
                        toggle(command.key, value === true)
                      }
                    />
                    <Label
                      htmlFor={`imported-${command.key}`}
                      className="cursor-pointer text-[13px] font-medium"
                    >
                      {command.label}
                    </Label>
                  </div>
                  <pre className="mt-2 overflow-x-auto whitespace-pre-wrap break-all rounded-md bg-background/60 p-2 font-mono text-[11px] text-muted-foreground">
                    {formatCommandValue(command.value) || "(empty)"}
                  </pre>
                </li>
              );
            })}
          </ul>
        </ScrollArea>

        <DialogFooter>
          <Button variant="ghost" disabled={busy} onClick={discardAll}>
            Discard all
          </Button>
          <Button disabled={busy || selected.size === 0} onClick={applySelected}>
            {busy
              ? "Applying…"
              : selected.size === 0
                ? "Apply selected"
                : `Apply selected (${selected.size})`}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
