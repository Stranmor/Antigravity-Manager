import { create } from 'zustand';
import { useShallow } from 'zustand/react/shallow';
import { AppConfig } from '../types/config';
import * as configService from '../services/configService';

// =============================================================================
// State Types (Separated from Actions for SOTA 2026 patterns)
// =============================================================================

interface ConfigStateSlice {
    config: AppConfig | null;
    loading: boolean;
    error: string | null;
}

interface ConfigActionsSlice {
    loadConfig: () => Promise<void>;
    saveConfig: (config: AppConfig) => Promise<void>;
    updateTheme: (theme: string) => Promise<void>;
    updateLanguage: (language: string) => Promise<void>;
}

type ConfigState = ConfigStateSlice & ConfigActionsSlice;

// =============================================================================
// Store Implementation
// =============================================================================

export const useConfigStore = create<ConfigState>((set, get) => ({
    // Initial State
    config: null,
    loading: false,
    error: null,

    // Actions
    loadConfig: async () => {
        set({ loading: true, error: null });
        try {
            const config = await configService.loadConfig();
            set({ config, loading: false });
        } catch (error) {
            set({ error: String(error), loading: false });
        }
    },

    saveConfig: async (config: AppConfig) => {
        set({ loading: true, error: null });
        try {
            await configService.saveConfig(config);
            set({ config, loading: false });
        } catch (error) {
            set({ error: String(error), loading: false });
            throw error;
        }
    },

    updateTheme: async (theme: string) => {
        const { config } = get();
        if (!config) return;

        const newConfig = { ...config, theme };
        await get().saveConfig(newConfig);
    },

    updateLanguage: async (language: string) => {
        const { config } = get();
        if (!config) return;

        const newConfig = { ...config, language };
        await get().saveConfig(newConfig);
    },
}));

// =============================================================================
// Atomic Selectors (SOTA 2026: Prevents unnecessary re-renders)
// =============================================================================

/** Select config object - only re-renders when config changes */
export const useConfig = () => useConfigStore(state => state.config);

/** Select loading state - only re-renders when loading changes */
export const useConfigLoading = () => useConfigStore(state => state.loading);

/** Select error state - only re-renders when error changes */
export const useConfigError = () => useConfigStore(state => state.error);

// =============================================================================
// Derived Selectors (Specific config values)
// =============================================================================

/** Select theme - only re-renders when theme changes */
export const useTheme = () => useConfigStore(state => state.config?.theme);

/** Select language - only re-renders when language changes */
export const useLanguage = () => useConfigStore(state => state.config?.language);

/** Select proxy config - only re-renders when proxy config changes */
export const useProxyConfig = () => useConfigStore(state => state.config?.proxy);

/** Select auto refresh settings */
export const useAutoRefreshSettings = () => useConfigStore(
    useShallow(state => ({
        autoRefresh: state.config?.auto_refresh ?? false,
        refreshInterval: state.config?.refresh_interval ?? 15,
    }))
);

/** Select auto sync settings */
export const useAutoSyncSettings = () => useConfigStore(
    useShallow(state => ({
        autoSync: state.config?.auto_sync ?? false,
        syncInterval: state.config?.sync_interval ?? 5,
    }))
);

// =============================================================================
// Composite Selectors with useShallow
// =============================================================================

/** Select config and loading state together */
export const useConfigWithStatus = () => useConfigStore(
    useShallow(state => ({
        config: state.config,
        loading: state.loading,
        error: state.error,
    }))
);

// =============================================================================
// Action Selectors (Stable references - never cause re-renders)
// =============================================================================

/** Get all config actions - stable reference */
export const useConfigActions = () => useConfigStore(
    useShallow(state => ({
        loadConfig: state.loadConfig,
        saveConfig: state.saveConfig,
        updateTheme: state.updateTheme,
        updateLanguage: state.updateLanguage,
    }))
);
