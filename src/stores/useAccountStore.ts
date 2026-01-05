import { create } from 'zustand';
import { useShallow } from 'zustand/react/shallow';
import { Account } from '../types/account';
import * as accountService from '../services/accountService';

// =============================================================================
// State Types (Separated from Actions for SOTA 2026 patterns)
// =============================================================================

interface AccountStateSlice {
    accounts: Account[];
    currentAccount: Account | null;
    loading: boolean;
    error: string | null;
}

interface AccountActionsSlice {
    fetchAccounts: () => Promise<void>;
    fetchCurrentAccount: () => Promise<void>;
    addAccount: (email: string, refreshToken: string) => Promise<void>;
    deleteAccount: (accountId: string) => Promise<void>;
    deleteAccounts: (accountIds: string[]) => Promise<void>;
    switchAccount: (accountId: string) => Promise<void>;
    refreshQuota: (accountId: string) => Promise<void>;
    refreshAllQuotas: () => Promise<accountService.RefreshStats>;
    reorderAccounts: (accountIds: string[]) => Promise<void>;
    startOAuthLogin: () => Promise<void>;
    completeOAuthLogin: () => Promise<void>;
    cancelOAuthLogin: () => Promise<void>;
    importV1Accounts: () => Promise<void>;
    importFromDb: () => Promise<void>;
    importFromCustomDb: (path: string) => Promise<void>;
    syncAccountFromDb: () => Promise<void>;
    toggleProxyStatus: (accountId: string, enable: boolean, reason?: string) => Promise<void>;
}

type AccountState = AccountStateSlice & AccountActionsSlice;

// =============================================================================
// Store Implementation
// =============================================================================

export const useAccountStore = create<AccountState>((set, get) => ({
    // Initial State
    accounts: [],
    currentAccount: null,
    loading: false,
    error: null,

    // Actions (kept stable, don't cause re-renders when state changes)
    fetchAccounts: async () => {
        set({ loading: true, error: null });
        try {
            console.warn('[Store] Fetching accounts...');
            const accounts = await accountService.listAccounts();
            set({ accounts, loading: false });
        } catch (error) {
            console.error('[Store] Fetch accounts failed:', error);
            set({ error: String(error), loading: false });
        }
    },

    fetchCurrentAccount: async () => {
        set({ loading: true, error: null });
        try {
            const account = await accountService.getCurrentAccount();
            set({ currentAccount: account, loading: false });
        } catch (error) {
            set({ error: String(error), loading: false });
        }
    },

    addAccount: async (email: string, refreshToken: string) => {
        set({ loading: true, error: null });
        try {
            await accountService.addAccount(email, refreshToken);
            await get().fetchAccounts();
            set({ loading: false });
        } catch (error) {
            set({ error: String(error), loading: false });
            throw error;
        }
    },

    deleteAccount: async (accountId: string) => {
        set({ loading: true, error: null });
        try {
            await accountService.deleteAccount(accountId);
            await Promise.all([
                get().fetchAccounts(),
                get().fetchCurrentAccount()
            ]);
            set({ loading: false });
        } catch (error) {
            set({ error: String(error), loading: false });
            throw error;
        }
    },

    deleteAccounts: async (accountIds: string[]) => {
        set({ loading: true, error: null });
        try {
            await accountService.deleteAccounts(accountIds);
            await Promise.all([
                get().fetchAccounts(),
                get().fetchCurrentAccount()
            ]);
            set({ loading: false });
        } catch (error) {
            set({ error: String(error), loading: false });
            throw error;
        }
    },

    switchAccount: async (accountId: string) => {
        set({ loading: true, error: null });
        try {
            await accountService.switchAccount(accountId);
            await get().fetchCurrentAccount();
            set({ loading: false });
        } catch (error) {
            set({ error: String(error), loading: false });
            throw error;
        }
    },

    refreshQuota: async (accountId: string) => {
        set({ loading: true, error: null });
        try {
            await accountService.fetchAccountQuota(accountId);
            await get().fetchAccounts();
            set({ loading: false });
        } catch (error) {
            set({ error: String(error), loading: false });
            throw error;
        }
    },

    refreshAllQuotas: async () => {
        set({ loading: true, error: null });
        try {
            const stats = await accountService.refreshAllQuotas();
            await get().fetchAccounts();
            set({ loading: false });
            return stats;
        } catch (error) {
            set({ error: String(error), loading: false });
            throw error;
        }
    },

    reorderAccounts: async (accountIds: string[]) => {
        const { accounts } = get();

        const accountMap = new Map(accounts.map(acc => [acc.id, acc]));
        const reorderedAccounts = accountIds
            .map(id => accountMap.get(id))
            .filter((acc): acc is Account => acc !== undefined);

        const remainingAccounts = accounts.filter(acc => !accountIds.includes(acc.id));
        const finalAccounts = [...reorderedAccounts, ...remainingAccounts];

        // Optimistic update
        set({ accounts: finalAccounts });

        try {
            await accountService.reorderAccounts(accountIds);
        } catch (error) {
            // Rollback on failure
            console.error('[AccountStore] Reorder accounts failed:', error);
            set({ accounts });
            throw error;
        }
    },

    startOAuthLogin: async () => {
        set({ loading: true, error: null });
        try {
            await accountService.startOAuthLogin();
            await get().fetchAccounts();
            set({ loading: false });
        } catch (error) {
            set({ error: String(error), loading: false });
            throw error;
        }
    },

    completeOAuthLogin: async () => {
        set({ loading: true, error: null });
        try {
            await accountService.completeOAuthLogin();
            await get().fetchAccounts();
            set({ loading: false });
        } catch (error) {
            set({ error: String(error), loading: false });
            throw error;
        }
    },

    cancelOAuthLogin: async () => {
        try {
            await accountService.cancelOAuthLogin();
            set({ loading: false, error: null });
        } catch (error) {
            console.error('[Store] Cancel OAuth failed:', error);
        }
    },

    importV1Accounts: async () => {
        set({ loading: true, error: null });
        try {
            await accountService.importV1Accounts();
            await get().fetchAccounts();
            set({ loading: false });
        } catch (error) {
            set({ error: String(error), loading: false });
            throw error;
        }
    },

    importFromDb: async () => {
        set({ loading: true, error: null });
        try {
            await accountService.importFromDb();
            await Promise.all([
                get().fetchAccounts(),
                get().fetchCurrentAccount()
            ]);
            set({ loading: false });
        } catch (error) {
            set({ error: String(error), loading: false });
            throw error;
        }
    },

    importFromCustomDb: async (path: string) => {
        set({ loading: true, error: null });
        try {
            await accountService.importFromCustomDb(path);
            await Promise.all([
                get().fetchAccounts(),
                get().fetchCurrentAccount()
            ]);
            set({ loading: false });
        } catch (error) {
            set({ error: String(error), loading: false });
            throw error;
        }
    },

    syncAccountFromDb: async () => {
        try {
            const syncedAccount = await accountService.syncAccountFromDb();
            if (syncedAccount) {
                console.warn('[AccountStore] Account synced from DB:', syncedAccount.email);
                await get().fetchAccounts();
                set({ currentAccount: syncedAccount });
            }
        } catch (error) {
            console.error('[AccountStore] Sync from DB failed:', error);
        }
    },

    toggleProxyStatus: async (accountId: string, enable: boolean, reason?: string) => {
        try {
            await accountService.toggleProxyStatus(accountId, enable, reason);
            await get().fetchAccounts();
        } catch (error) {
            console.error('[AccountStore] Toggle proxy status failed:', error);
            throw error;
        }
    },
}));

// =============================================================================
// Atomic Selectors (SOTA 2026: Prevents unnecessary re-renders)
// =============================================================================

/** Select accounts array - only re-renders when accounts change */
export const useAccounts = () => useAccountStore(state => state.accounts);

/** Select current account - only re-renders when currentAccount changes */
export const useCurrentAccount = () => useAccountStore(state => state.currentAccount);

/** Select loading state - only re-renders when loading changes */
export const useAccountLoading = () => useAccountStore(state => state.loading);

/** Select error state - only re-renders when error changes */
export const useAccountError = () => useAccountStore(state => state.error);

// =============================================================================
// Composite Selectors with useShallow (for multiple primitive values)
// =============================================================================

/** Select accounts and currentAccount together with shallow comparison */
export const useAccountData = () => useAccountStore(
    useShallow(state => ({
        accounts: state.accounts,
        currentAccount: state.currentAccount,
    }))
);

/** Select loading/error status together */
export const useAccountStatus = () => useAccountStore(
    useShallow(state => ({
        loading: state.loading,
        error: state.error,
    }))
);

// =============================================================================
// Action Selectors (Stable references - never cause re-renders)
// =============================================================================

/** Get all account actions - stable reference, never causes re-renders */
export const useAccountActions = () => useAccountStore(
    useShallow(state => ({
        fetchAccounts: state.fetchAccounts,
        fetchCurrentAccount: state.fetchCurrentAccount,
        addAccount: state.addAccount,
        deleteAccount: state.deleteAccount,
        deleteAccounts: state.deleteAccounts,
        switchAccount: state.switchAccount,
        refreshQuota: state.refreshQuota,
        refreshAllQuotas: state.refreshAllQuotas,
        reorderAccounts: state.reorderAccounts,
        startOAuthLogin: state.startOAuthLogin,
        completeOAuthLogin: state.completeOAuthLogin,
        cancelOAuthLogin: state.cancelOAuthLogin,
        importV1Accounts: state.importV1Accounts,
        importFromDb: state.importFromDb,
        importFromCustomDb: state.importFromCustomDb,
        syncAccountFromDb: state.syncAccountFromDb,
        toggleProxyStatus: state.toggleProxyStatus,
    }))
);

/** Get fetch actions only */
export const useFetchActions = () => useAccountStore(
    useShallow(state => ({
        fetchAccounts: state.fetchAccounts,
        fetchCurrentAccount: state.fetchCurrentAccount,
    }))
);

/** Get OAuth actions only */
export const useOAuthActions = () => useAccountStore(
    useShallow(state => ({
        startOAuthLogin: state.startOAuthLogin,
        completeOAuthLogin: state.completeOAuthLogin,
        cancelOAuthLogin: state.cancelOAuthLogin,
    }))
);

/** Get import actions only */
export const useImportActions = () => useAccountStore(
    useShallow(state => ({
        importV1Accounts: state.importV1Accounts,
        importFromDb: state.importFromDb,
        importFromCustomDb: state.importFromCustomDb,
    }))
);
