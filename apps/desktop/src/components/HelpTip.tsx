import {useEffect,useId,useLayoutEffect,useRef,useState} from 'react';
import {createPortal} from 'react-dom';
import {CircleHelp} from 'lucide-react';

/** A small explanation that also works with a keyboard or a touch screen. */
export function HelpTip({label,children}:{label:string;children:React.ReactNode}) {
  const id=useId(),button=useRef<HTMLButtonElement>(null),tip=useRef<HTMLDivElement>(null);
  const timer=useRef<ReturnType<typeof setTimeout>|null>(null),pinned=useRef(false);
  const [open,setOpen]=useState(false),[position,setPosition]=useState({left:8,top:8});
  function clear(){if(timer.current)clearTimeout(timer.current);timer.current=null;}
  function close(){clear();pinned.current=false;setOpen(false);}
  function leave(){clear();if(!pinned.current)timer.current=setTimeout(()=>setOpen(false),150);}
  useEffect(()=>()=>clear(),[]);
  useEffect(()=>{if(!open)return;const escape=(e:KeyboardEvent)=>{if(e.key==='Escape')close();};const outside=(e:PointerEvent)=>{if(!button.current?.contains(e.target as Node)&&!tip.current?.contains(e.target as Node))close();};document.addEventListener('keydown',escape);document.addEventListener('pointerdown',outside);return()=>{document.removeEventListener('keydown',escape);document.removeEventListener('pointerdown',outside);};},[open]);
  useLayoutEffect(()=>{if(!open)return;const locate=()=>{const b=button.current?.getBoundingClientRect(),t=tip.current?.getBoundingClientRect();if(!b||!t)return;setPosition({left:Math.max(8,Math.min(b.left,innerWidth-t.width-8)),top:b.bottom+8+t.height<=innerHeight-8?b.bottom+8:Math.max(8,b.top-t.height-8)});};locate();window.addEventListener('resize',locate);window.addEventListener('scroll',locate,true);return()=>{window.removeEventListener('resize',locate);window.removeEventListener('scroll',locate,true);};},[open]);
  return <><button ref={button} type="button" className="help-trigger" aria-label={label} aria-expanded={open} aria-describedby={open?id:undefined}
    onPointerEnter={e=>{if(e.pointerType==='mouse'){clear();timer.current=setTimeout(()=>setOpen(true),250);}}} onPointerLeave={leave}
    onFocus={()=>{clear();setOpen(true);}} onBlur={()=>{if(!pinned.current)close();}}
    onClick={()=>{clear();pinned.current=!pinned.current;setOpen(pinned.current);}}><CircleHelp size={16}/></button>
    {open&&createPortal(<div ref={tip} id={id} role="tooltip" className="help-content" style={position} onPointerEnter={clear} onPointerLeave={leave}>{children}</div>,document.body)}</>;
}
