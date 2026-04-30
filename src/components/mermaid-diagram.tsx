import { useEffect, useRef, useState } from "react"

interface MermaidDiagramProps {
  code: string
}

export function MermaidDiagram({ code }: MermaidDiagramProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const [error, setError] = useState<string | null>(null)
  const [svg, setSvg] = useState<string | null>(null)
  const [visible, setVisible] = useState(false)

  // Only render when the diagram scrolls into view
  useEffect(() => {
    const el = containerRef.current
    if (!el) return

    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) {
          setVisible(true)
          observer.disconnect()
        }
      },
      { rootMargin: "200px" },
    )
    observer.observe(el)
    return () => observer.disconnect()
  }, [code])

  // Render mermaid SVG once visible
  useEffect(() => {
    if (!visible) return
    let cancelled = false

    async function render() {
      try {
        const mermaid = (await import("mermaid")).default
        mermaid.initialize({
          startOnLoad: false,
          theme: "default",
          securityLevel: "strict",
        })

        const id = `mermaid-${Math.random().toString(36).slice(2, 10)}`
        const { svg: rendered } = await mermaid.render(id, code)
        if (!cancelled) {
          setSvg(rendered)
          setError(null)
        }
      } catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : String(err))
          setSvg(null)
        }
      }
    }

    render()
    return () => { cancelled = true }
  }, [visible, code])

  // Prevent layout shift: compute a stable min-height from code line count
  const estimatedHeight = Math.max(80, code.split("\n").length * 20)

  if (error) {
    return (
      <div className="my-2 rounded border border-red-300/60 bg-red-50/50 dark:bg-red-950/20 p-2 text-xs text-red-700 dark:text-red-400">
        <p className="font-medium mb-1">Mermaid syntax error</p>
        <pre className="whitespace-pre-wrap text-[11px] opacity-70">{error}</pre>
      </div>
    )
  }

  return (
    <div
      ref={containerRef}
      className="my-2 overflow-x-auto rounded border border-border/40 bg-muted/20 [&>svg]:mx-auto [&>svg]:max-w-full [&>svg]:h-auto"
      style={{ minHeight: svg ? undefined : estimatedHeight }}
    >
      {svg ? (
        <div className="p-3" dangerouslySetInnerHTML={{ __html: svg }} />
      ) : visible ? (
        <div className="flex items-center justify-center h-full p-4">
          <span className="text-xs text-muted-foreground animate-pulse">Rendering diagram...</span>
        </div>
      ) : (
        <div className="flex items-center justify-center h-full p-4">
          <span className="text-xs text-muted-foreground/50">Diagram</span>
        </div>
      )}
    </div>
  )
}
