import React from 'react';
import { Terminal, ChevronRight } from 'lucide-react';
import { ModuleId, ModuleData } from '../data/modules';

interface SidebarProps {
  activeModule: ModuleId;
  setActiveModule: (id: ModuleId) => void;
  modules: ModuleData[];
}

export function Sidebar({ activeModule, setActiveModule, modules }: SidebarProps) {
  return (
    <aside className="w-full md:w-72 bg-[#0a0a0a] border-r border-zinc-800 flex flex-col shrink-0">
      <div className="p-6 border-b border-zinc-800 flex items-center gap-3">
        <Terminal className="w-6 h-6 text-orange-500" />
        <div>
          <h1 className="font-bold text-white tracking-tight">OpticC</h1>
          <p className="text-xs text-zinc-500 font-mono">Compiler Source & Docs</p>
        </div>
      </div>

      <nav className="flex-1 overflow-y-auto py-4 px-3 space-y-1">
        {modules.map((mod) => {
          const isActive = activeModule === mod.id;
          return (
            <button
              key={mod.id}
              onClick={() => setActiveModule(mod.id)}
              className={`w-full flex items-center gap-3 px-3 py-2.5 rounded-lg text-left transition-all duration-200 ${
                isActive
                  ? 'bg-orange-500/10 text-orange-400'
                  : 'text-zinc-400 hover:bg-zinc-900 hover:text-zinc-200'
              }`}
            >
              <div className={`${isActive ? 'text-orange-500' : 'text-zinc-500'}`}>
                {mod.icon}
              </div>
              <span className="font-medium text-sm flex-1">{mod.title}</span>
              {isActive && <ChevronRight className="w-4 h-4 text-orange-500" />}
            </button>
          );
        })}
      </nav>

      <div className="p-4 border-t border-zinc-800">
        <div className="bg-zinc-900 rounded-lg p-3 border border-zinc-800">
          <p className="text-xs text-zinc-500 font-mono mb-1">Target</p>
          <p className="text-sm text-zinc-300 font-medium">C99 / C11</p>
          <div className="mt-2 h-1 w-full bg-zinc-800 rounded-full overflow-hidden">
            <div className="h-full bg-orange-500 w-1/3"></div>
          </div>
        </div>
      </div>
    </aside>
  );
}
