import React from 'react';
import { motion, AnimatePresence } from 'motion/react';
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter';
import { vscDarkPlus } from 'react-syntax-highlighter/dist/esm/styles/prism';
import { FileCode2 } from 'lucide-react';
import { ModuleData } from '../data/modules';

interface ContentProps {
  currentData: ModuleData;
}

export function Content({ currentData }: ContentProps) {
  return (
    <main className="flex-1 flex flex-col h-screen overflow-hidden bg-[#050505]">

      {/* Topbar */}
      <header className="h-16 border-b border-zinc-800 flex items-center px-8 shrink-0 bg-[#0a0a0a]/50 backdrop-blur-md">
        <h2 className="text-lg font-semibold text-white flex items-center gap-2">
          {currentData.icon}
          {currentData.title}
        </h2>
      </header>

      {/* Scrollable Content */}
      <div className="flex-1 overflow-y-auto p-8">
        <div className="max-w-4xl mx-auto">

          <AnimatePresence mode="wait">
            <motion.div
              key={currentData.id}
              initial={{ opacity: 0, y: 10 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -10 }}
              transition={{ duration: 0.2 }}
              className="space-y-12"
            >
              {/* Documentation Section */}
              <section>
                <h1 className="text-3xl font-bold text-white mb-2">{currentData.title}</h1>
                <p className="text-lg text-zinc-400 mb-8 pb-8 border-b border-zinc-800">
                  {currentData.description}
                </p>
                <div className="prose prose-invert prose-orange max-w-none">
                  {currentData.content}
                </div>
              </section>

              {/* Code Section */}
              {currentData.code && (
                <section className="mt-12">
                  <div className="flex items-center justify-between mb-4">
                    <h3 className="text-xl font-semibold text-white flex items-center gap-2">
                      <FileCode2 className="w-5 h-5 text-orange-500" />
                      Implementation
                    </h3>
                    <div className="bg-zinc-900 border border-zinc-800 px-3 py-1 rounded-md flex items-center gap-2">
                      <span className="w-2 h-2 rounded-full bg-orange-500"></span>
                      <span className="text-xs font-mono text-zinc-400">{currentData.code.filename}</span>
                    </div>
                  </div>

                  <div className="rounded-xl overflow-hidden border border-zinc-800 shadow-2xl">
                    <SyntaxHighlighter
                      language={currentData.code.language}
                      style={vscDarkPlus}
                      customStyle={{
                        margin: 0,
                        padding: '1.5rem',
                        background: '#0a0a0a',
                        fontSize: '0.875rem',
                        lineHeight: '1.6',
                      }}
                      showLineNumbers={true}
                      lineNumberStyle={{
                        minWidth: '3em',
                        paddingRight: '1em',
                        color: '#404040',
                        textAlign: 'right',
                      }}
                    >
                      {currentData.code.snippet}
                    </SyntaxHighlighter>
                  </div>
                </section>
              )}
            </motion.div>
          </AnimatePresence>

        </div>
      </div>
    </main>
  );
}
