import { ContextMenu,ContextMenuTrigger,ContextMenuContent,ContextMenuItem } from "@/components/ui/context-menu";
import { AlertDialog,AlertDialogContent,AlertDialogHeader,AlertDialogTitle,AlertDialogDescription,AlertDialogFooter,AlertDialogCancel,AlertDialogAction } from "@/components/ui/alert-dialog";
import { bytes,processMatches,type SystemSnapshot } from "@/modules/desktop/model";
import { invoke } from "@tauri-apps/api/core";
import { useEffect,useState } from "react";
import { toast } from "sonner";
type Process=SystemSnapshot["processes"][number];
type Suspended={pid:number;started:number;name:string};
type ControlAction="terminate"|"kill"|"suspend"|"resume";
export function Processes({processes,refresh}:{processes:Process[];refresh:()=>void}){
  const [suspended,setSuspended]=useState<Suspended[]>([]);
  const [killing,setKilling]=useState<Process|null>(null);
  const [busy,setBusy]=useState(false);
  const [query,setQuery]=useState("");
  useEffect(()=>{let alive=true;void invoke<Suspended[]>("suspended_processes").then(value=>{if(alive)setSuspended(value??[]);}).catch(()=>{});return()=>{alive=false;};},[processes]);
  const suspendedPids=new Set(suspended.map(process=>process.pid));
  const visible=processes.filter(process=>processMatches(process,query));
  const control=async(process:{pid:number;started:number},action:ControlAction)=>{
    if(busy)return;setBusy(true);
    try{setSuspended(await invoke<Suspended[]>("control_process",{pid:process.pid,started:process.started,action}));setKilling(null);refresh();}catch(e){toast.error(String(e));}finally{setBusy(false);}
  };
  return <>
    <input type="search" value={query} onChange={event=>setQuery(event.target.value)} placeholder="FILTER PID / NAME" aria-label="Filter processes" className="mb-1 h-6 w-full rounded-md border border-border bg-background px-2 text-[9px] text-foreground outline-none placeholder:text-muted-foreground focus:border-foreground/40"/>
    <table className="tarex-processes"><thead><tr><th>PID</th><th>NAME</th><th>CPU</th><th>MEM</th></tr></thead><tbody>{visible.map(process=><ContextMenu key={`${process.pid}-${process.started}`}><ContextMenuTrigger asChild><tr tabIndex={0} title={`${process.name}\nPID ${process.pid}\nCPU ${process.cpu.toFixed(1)}%\nMemory ${bytes(process.memory)}`}><td>{process.pid}</td><td>{process.name}</td><td>{process.cpu.toFixed(1)}%</td><td>{bytes(process.memory)}</td></tr></ContextMenuTrigger><ContextMenuContent className="rounded-sm border border-border"><ContextMenuItem disabled={busy||suspendedPids.has(process.pid)} onSelect={()=>void control(process,"suspend")}>Suspend {process.name}</ContextMenuItem>{suspendedPids.has(process.pid)&&<ContextMenuItem disabled={busy} onSelect={()=>void control(process,"resume")}>Resume {process.name}</ContextMenuItem>}<ContextMenuItem disabled={busy} onSelect={()=>void control(process,"terminate")}>Terminate {process.name}</ContextMenuItem><ContextMenuItem variant="destructive" disabled={busy} onSelect={()=>setKilling(process)}>Force kill {process.name}</ContextMenuItem></ContextMenuContent></ContextMenu>)}</tbody></table>
    {visible.length===0&&<p style={{fontSize:"8px",textAlign:"center",color:"var(--tarex-dim)"}}>{processes.length===0?"NO PROCESSES":"NO MATCHES"}</p>}
    {suspended.length>0&&<section className="tarex-suspended" aria-label="Suspended processes"><div className="tarex-rule"><span>SUSPENDED</span><span>{suspended.length}</span></div>{suspended.map(process=><div key={`${process.pid}-${process.started}`} title={`${process.name} / PID ${process.pid}`}><span>{process.pid} {process.name}</span><button type="button" disabled={busy} onClick={()=>void control(process,"resume")}>Unsuspend</button></div>)}</section>}
    <AlertDialog open={!!killing} onOpenChange={open=>{if(!open&&!busy)setKilling(null);}}><AlertDialogContent><AlertDialogHeader><AlertDialogTitle>Force kill {killing?.name}?</AlertDialogTitle><AlertDialogDescription>Force kill process {killing?.pid} immediately? Unsaved work in that process may be lost. Use Terminate first for a graceful close.</AlertDialogDescription></AlertDialogHeader><AlertDialogFooter><AlertDialogCancel disabled={busy}>Cancel</AlertDialogCancel><AlertDialogAction disabled={busy} onClick={e=>{e.preventDefault();if(killing)void control(killing,"kill");}}>Force kill process</AlertDialogAction></AlertDialogFooter></AlertDialogContent></AlertDialog>
  </>;
}
