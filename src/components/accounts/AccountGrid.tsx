import { memo, useCallback } from 'react';
import { Account } from '../../types/account';
import AccountCard from './AccountCard';

interface AccountGridProps {
    accounts: Account[];
    selectedIds: Set<string>;
    refreshingIds: Set<string>;
    onToggleSelect: (id: string) => void;
    currentAccountId: string | null;
    switchingAccountId: string | null;
    onSwitch: (accountId: string) => void;
    onRefresh: (accountId: string) => void;
    onViewDetails: (accountId: string) => void;
    onExport: (accountId: string) => void;
    onDelete: (accountId: string) => void;
    onToggleProxy: (accountId: string) => void;
}


function AccountGrid({ accounts, selectedIds, refreshingIds, onToggleSelect, currentAccountId, switchingAccountId, onSwitch, onRefresh, onViewDetails, onExport, onDelete, onToggleProxy }: AccountGridProps) {
    const createSelectHandler = useCallback((id: string) => () => { onToggleSelect(id); }, [onToggleSelect]);
    const createSwitchHandler = useCallback((id: string) => () => { onSwitch(id); }, [onSwitch]);
    const createRefreshHandler = useCallback((id: string) => () => { onRefresh(id); }, [onRefresh]);
    const createDetailsHandler = useCallback((id: string) => () => { onViewDetails(id); }, [onViewDetails]);
    const createExportHandler = useCallback((id: string) => () => { onExport(id); }, [onExport]);
    const createDeleteHandler = useCallback((id: string) => () => { onDelete(id); }, [onDelete]);
    const createProxyHandler = useCallback((id: string) => () => { onToggleProxy(id); }, [onToggleProxy]);
    
    if (accounts.length === 0) {
        return (
            <div className="bg-white dark:bg-base-100 rounded-2xl p-12 shadow-sm border border-gray-100 dark:border-base-200 text-center">
                <p className="text-gray-400 mb-2">暂无账号</p>
                <p className="text-sm text-gray-400">点击上方"添加账号"按钮添加第一个账号</p>
            </div>
        );
    }

    return (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-4">
            {accounts.map((account) => (
                <AccountCard
                    key={account.id}
                    account={account}
                    selected={selectedIds.has(account.id)}
                    isRefreshing={refreshingIds.has(account.id)}
                    onSelect={createSelectHandler(account.id)}
                    isCurrent={account.id === currentAccountId}
                    isSwitching={account.id === switchingAccountId}
                    onSwitch={createSwitchHandler(account.id)}
                    onRefresh={createRefreshHandler(account.id)}
                    onViewDetails={createDetailsHandler(account.id)}
                    onExport={createExportHandler(account.id)}
                    onDelete={createDeleteHandler(account.id)}
                    onToggleProxy={createProxyHandler(account.id)}
                />
            ))}
        </div>
    );
}

export default memo(AccountGrid);
