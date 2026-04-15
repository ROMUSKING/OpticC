import { useState } from 'react';
import { Sidebar } from './components/Sidebar';
import { Content } from './components/Content';
import { modules, ModuleId } from './data/modules';

export default function App() {
  const [activeModule, setActiveModule] = useState<ModuleId>('architecture');

  const currentData = modules.find(m => m.id === activeModule)!;

  return (
    <div className="min-h-screen bg-[#050505] text-zinc-300 font-sans flex flex-col md:flex-row selection:bg-orange-500/30">
      <Sidebar
        activeModule={activeModule}
        setActiveModule={setActiveModule}
        modules={modules}
      />
      <Content currentData={currentData} />
    </div>
  );
}
