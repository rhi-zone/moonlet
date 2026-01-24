import { defineConfig } from 'vitepress'
import { withMermaid } from 'vitepress-plugin-mermaid'

export default withMermaid(
  defineConfig({
    title: 'moonlet',
    description: 'Agentic AI framework with Lua scripting',

    base: '/moonlet/',

    themeConfig: {
      nav: [
        { text: 'Guide', link: '/introduction' },
        { text: 'API', link: '/api' },
      ],

      sidebar: [
        {
          text: 'Guide',
          items: [
            { text: 'Introduction', link: '/introduction' },
            { text: 'Getting Started', link: '/getting-started' },
          ]
        },
        {
          text: 'Core',
          items: [
            { text: 'LLM Client', link: '/llm-client' },
            { text: 'Memory Store', link: '/memory' },
          ]
        },
        {
          text: 'Sessions',
          items: [
            { text: 'Session Parsing', link: '/sessions' },
            { text: 'Log Formats', link: '/log-formats' },
          ]
        },
        {
          text: 'Agent',
          items: [
            { text: 'Lua Scripts', link: '/agent-scripts' },
            { text: 'State Machine', link: '/state-machine' },
          ]
        },
      ],

      socialLinks: [
        { icon: 'github', link: 'https://github.com/rhi-zone/moonlet' }
      ],

      search: {
        provider: 'local'
      },

      editLink: {
        pattern: 'https://github.com/rhi-zone/moonlet/edit/master/docs/:path',
        text: 'Edit this page on GitHub'
      },
    },

    vite: {
      optimizeDeps: {
        include: ['mermaid'],
      },
    },
  }),
)
