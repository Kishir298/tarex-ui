// biome-ignore-all lint/suspicious/noArrayIndexKey: Hardware core indexes and fixed memory cells never reorder.
import {
  telemetryIntervalMs,
  textOr,
  type SystemSnapshot,
  usagePair,
} from "@/modules/desktop/model";
import { Processes } from "@/modules/desktop/Processes";
import { usePreferencesStore } from "@/modules/settings/preferences";
import { subscribeWindowPresentation } from "@/modules/terminal/ghostty/windowPresentation";
import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";

type Sample = { cpu: number };

export function useTelemetry(enabled = true) {
  const [snapshot, setSnapshot] = useState<SystemSnapshot | null>(null);
  const [history, setHistory] = useState<Sample[]>([]);
  const [error, setError] = useState("");
  const [revision,setRevision]=useState(0);
  const refresh=useCallback(()=>setRevision(value=>value+1),[]);
  const efficiencyMode = usePreferencesStore((s) => s.efficiencyMode);
  const prefsHydrated = usePreferencesStore((s) => s.hydrated);
  // Apply Efficiency Mode to Tarex's own process priority once the persisted
  // preference is known. Toggling off is driven by the Settings switch, which
  // reports the OS answer directly.
  useEffect(() => {
    if (!prefsHydrated || !efficiencyMode) return;
    void invoke("efficiency_mode_set", { enabled: true }).catch((e) =>
      console.warn("efficiency mode could not lower Tarex priority", e),
    );
  }, [prefsHydrated, efficiencyMode]);
  useEffect(() => {
    if (!enabled) return;
    let disposed = false;
    let pending = false;
    let visible = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let forceDetails=revision>0;
    const sample = async () => {
      if (disposed || pending || !visible) return;
      pending = true;
      try {
        const next = await invoke<SystemSnapshot>("dashboard_snapshot",{forceDetails});
        forceDetails=false;
        if (!disposed && visible) {
          setSnapshot(next);
          setError("");
          const cpu =
            next.cpu.reduce((sum, value) => sum + value, 0) /
            Math.max(1, next.cpu.length);
          setHistory((previous) => [
            ...previous.slice(-59),
            { cpu },
          ]);
        }
      } catch (e) {
        if (!disposed && visible) setError(String(e));
      } finally {
        pending = false;
        if (!disposed && visible)
          timer = setTimeout(sample, telemetryIntervalMs(efficiencyMode));
      }
    };
    const unsubscribe = subscribeWindowPresentation((state) => {
      visible = state.visible;
      clearTimeout(timer);
      if (visible) void sample();
    });
    return () => {
      disposed = true;
      clearTimeout(timer);
      unsubscribe();
    };
  }, [enabled, revision, efficiencyMode]);
  return { snapshot, history, error,refresh };
}

function Clock() {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    let timer: ReturnType<typeof setInterval> | undefined;
    const unsubscribe = subscribeWindowPresentation(({ visible }) => {
      clearInterval(timer);
      if (visible) {
        setNow(new Date());
        timer = setInterval(() => setNow(new Date()), 1000);
      }
    });
    return () => {
      clearInterval(timer);
      unsubscribe();
    };
  }, []);
  return (
    <>
      <time className="tarex-clock" dateTime={now.toISOString()}>
        {now.toLocaleTimeString("en-GB")}
      </time>
      <div className="tarex-date">
        {now
          .toLocaleDateString("en-US", {
            year: "numeric",
            month: "short",
            day: "2-digit",
            weekday: "short",
          })
          .toUpperCase()}
      </div>
    </>
  );
}

function Chart({
  values,
  second,
  max = 100,
  label,
}: {
  values: number[];
  second?: number[];
  max?: number;
  label: string;
}) {
  const line = (samples: number[]) =>
    samples
      .map(
        (value, index) =>
          `${200 - (samples.length - 1 - index) * (200 / 59)},${58 - (Math.min(max, Math.max(0, value)) / Math.max(1, max)) * 52}`,
      )
      .join(" ");
  return (
    <svg
      className="tarex-chart"
      viewBox="0 0 200 60"
      preserveAspectRatio="none"
      role="img"
      aria-label={label}
    >
      <title>{label}</title>
      <path
        className="tarex-chart-grid"
        d="M0 0H200M0 15H200M0 30H200M0 45H200M0 59H200M0 0V60M40 0V60M80 0V60M120 0V60M160 0V60M200 0V60"
      />
      <polyline
        points={line(values)}
        fill="none"
        stroke="currentColor"
        strokeWidth="1.3"
        vectorEffect="non-scaling-stroke"
      />
      {second && (
        <polyline
          points={line(second)}
          fill="none"
          stroke="var(--tarex-bright)"
          strokeWidth="1"
          opacity=".7"
          vectorEffect="non-scaling-stroke"
        />
      )}
    </svg>
  );
}

export function SystemRail({
  snapshot: s,
  history,
  error,
  refresh,
}: ReturnType<typeof useTelemetry>) {
  const average = s?.cpu.length
    ? s.cpu.reduce((a, b) => a + b, 0) / s.cpu.length
    : 0;
  const memory = s?.memoryTotal ? s.memoryUsed / s.memoryTotal : 0;
  const uptime = s
    ? `${Math.floor(s.uptime / 86400)}D ${String(Math.floor(s.uptime / 3600) % 24).padStart(2, "0")}:${String(Math.floor(s.uptime / 60) % 60).padStart(2, "0")}`
    : "--";
  return (
    <aside className="tarex-system-rail" aria-label="Live system monitor">
      <div className="tarex-rule">
        <span>PANEL</span>
        <span>SYSTEM</span>
      </div>
      <Clock />
      <dl className="tarex-machine-summary">
        <div>
          <dt>UPTIME</dt>
          <dd>{uptime}</dd>
        </div>
        <div>
          <dt>TYPE</dt>
          <dd>{textOr(s?.os)}</dd>
        </div>
        <div>
          <dt>ARCH</dt>
          <dd>{textOr(s?.architecture)}</dd>
        </div>
      </dl>
      <div className="tarex-hardware">
        <span>HOST / KERNEL</span>
        <strong title={s?.hostname ?? undefined}>{textOr(s?.hostname, "CONNECTING")}</strong>
        <span>{textOr(s?.kernel)}</span>
        <strong title={s?.cpuName ?? undefined}>{textOr(s?.cpuName, "Unavailable")}</strong>
      </div>
      <div className="tarex-rule">
        <span>CPU USAGE</span>
        <span>{s?.cpu.length ? `${average.toFixed(1)}%` : "--"}</span>
      </div>
      <div className="tarex-graph-caption">
        <span>AVG. {s?.cpu.length ? s.cpu.length : "--"} THREADS</span>
        <span>100%</span>
      </div>
      <Chart
        values={history.map((sample) => sample.cpu)}
        label={`CPU usage ${average.toFixed(1)} percent`}
      />
      <div className="tarex-core-bars">
        {s?.cpu.map((value, index) => (
          <div
            key={`core-${index}`}
            title={`Core ${index + 1}: ${value.toFixed(1)}%`}
          >
            <i style={{ height: `${Math.max(2, value)}%` }} />
          </div>
        ))}
      </div>
      <dl className="tarex-machine-summary">
        <div>
          <dt>CPU</dt>
          <dd>{s?.cpu.length ? `${average.toFixed(0)}%` : "--"}</dd>
        </div>
        <div>
          <dt>CORES</dt>
          <dd>{s?.cpu.length ? s.cpu.length : "--"}</dd>
        </div>
        <div>
          <dt>TASKS</dt>
          <dd>{s ? "TOP 5" : "--"}</dd>
        </div>
      </dl>
      <div className="tarex-rule">
        <span>MEMORY</span>
        <span>{usagePair(s?.memoryUsed, s?.memoryTotal)}</span>
      </div>
      <div
        className="tarex-memory-map"
        role="img"
        aria-label={
          s?.memoryTotal
            ? `Memory ${(memory * 100).toFixed(0)} percent used`
            : "Memory unavailable"
        }
      >
        {Array.from({ length: 200 }, (_, index) => (
          <i
            key={`cell-${index}`}
            className={index < memory * 200 ? "used" : ""}
          />
        ))}
      </div>
      <div className="tarex-rule">
        <span>SWAP</span>
        <span>{usagePair(s?.swapUsed, s?.swapTotal)}</span>
      </div>
      {s?.swapTotal ? (
        <div className="tarex-meter">
          <i
            style={{
              width: `${(s.swapUsed / s.swapTotal) * 100}%`,
            }}
          />
        </div>
      ) : null}
      <div className="tarex-rule">
        <span>TOP PROCESSES</span>
        <span>CPU / MEM</span>
      </div>
      <Processes processes={s?.processes??[]} refresh={refresh}/>
      {error && (
        <p role="status" className="tarex-error" title={error}>
          SYSTEM DATA UNAVAILABLE
        </p>
      )}
    </aside>
  );
}
