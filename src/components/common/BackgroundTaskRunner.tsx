import { useEffect, useRef, useCallback } from 'react';
import { useConfigStore } from '../../stores/useConfigStore';
import { useAccountStore } from '../../stores/useAccountStore';

function BackgroundTaskRunner() {
    const { config } = useConfigStore();
    const { refreshAllQuotas } = useAccountStore();

    // Use refs to track previous state to detect "off -> on" transitions
    const prevAutoRefreshRef = useRef(false);
    const prevAutoSyncRef = useRef(false);

    // Stable callback refs
    const refreshAllQuotasRef = useRef(refreshAllQuotas);
    refreshAllQuotasRef.current = refreshAllQuotas;

    const stableRefreshAllQuotas = useCallback(() => {
        void refreshAllQuotasRef.current();
    }, []);

    // Auto Refresh Quota Effect
    useEffect(() => {
        if (!config) return;

        let intervalId: ReturnType<typeof setTimeout> | null = null;
        const { auto_refresh, refresh_interval } = config;

        // Check if we just turned it on
        if (auto_refresh && !prevAutoRefreshRef.current) {
            stableRefreshAllQuotas();
        }
        prevAutoRefreshRef.current = auto_refresh;

        if (auto_refresh && refresh_interval > 0) {
            intervalId = setInterval(() => {
                stableRefreshAllQuotas();
            }, refresh_interval * 60 * 1000);
        }

        return () => {
            if (intervalId) {
                clearInterval(intervalId);
            }
        };
    }, [config?.auto_refresh, config?.refresh_interval, stableRefreshAllQuotas]);

    // Auto Sync Current Account Effect
    useEffect(() => {
        if (!config) return;

        let intervalId: ReturnType<typeof setTimeout> | null = null;
        const { auto_sync, sync_interval } = config;
        const { syncAccountFromDb } = useAccountStore.getState();

        const stableSyncAccountFromDb = () => {
            void syncAccountFromDb();
        };

        // Check if we just turned it on
        if (auto_sync && !prevAutoSyncRef.current) {
            stableSyncAccountFromDb();
        }
        prevAutoSyncRef.current = auto_sync;

        if (auto_sync && sync_interval > 0) {
            intervalId = setInterval(() => {
                stableSyncAccountFromDb();
            }, sync_interval * 1000);
        }

        return () => {
            if (intervalId) {
                clearInterval(intervalId);
            }
        };
    }, [config?.auto_sync, config?.sync_interval]);

    // Render nothing
    return null;
}

export default BackgroundTaskRunner;
