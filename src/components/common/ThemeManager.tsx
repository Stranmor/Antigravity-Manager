
import { useEffect, useCallback, useRef } from 'react';
import { useConfigStore } from '../../stores/useConfigStore';
import { getCurrentWindow } from '@tauri-apps/api/window';

export default function ThemeManager() {
    const { config, loadConfig } = useConfigStore();
    const windowShownRef = useRef(false);

    useEffect(() => {
        const init = async () => {
            await loadConfig();
            if (!windowShownRef.current) {
                windowShownRef.current = true;
                setTimeout(() => {
                    void getCurrentWindow().show();
                }, 100);
            }
        };
        void init();
    }, [loadConfig]);

    const applyTheme = useCallback(async (theme: string) => {
        const root = document.documentElement;
        const isDark = theme === 'dark';

        // Set Tauri window background color
        try {
            const bgColor = isDark ? '#1d232a' : '#FAFBFC';
            await getCurrentWindow().setBackgroundColor(bgColor);
        } catch (e) {
            console.error('Failed to set window background color:', e);
        }

        // Set DaisyUI theme
        root.setAttribute('data-theme', theme);

        // Set inline style for immediate visual feedback
        root.style.backgroundColor = isDark ? '#1d232a' : '#FAFBFC';

        // Set Tailwind dark mode class
        if (isDark) {
            root.classList.add('dark');
        } else {
            root.classList.remove('dark');
        }
    }, []);

    // Apply theme when config changes
    useEffect(() => {
        if (!config) return;

        const theme = config.theme || 'system';

        // Sync to localStorage for early boot check
        localStorage.setItem('app-theme-preference', theme);

        if (theme === 'system') {
            const mediaQuery = window.matchMedia('(prefers-color-scheme: dark)');

            const handleSystemChange = (e: MediaQueryListEvent | MediaQueryList) => {
                const systemTheme = e.matches ? 'dark' : 'light';
                void applyTheme(systemTheme);
            };

            // Initial alignment
            handleSystemChange(mediaQuery);

            // Listen for changes
            mediaQuery.addEventListener('change', handleSystemChange);
            return () => { mediaQuery.removeEventListener('change', handleSystemChange); };
        } else {
            void applyTheme(theme);
            return; // Explicit return for non-system theme path
        }
        // eslint-disable-next-line react-hooks/exhaustive-deps -- using optional chaining on config.theme, config itself not needed
    }, [config?.theme, applyTheme]);

    return null; // This component handles side effects only
}
