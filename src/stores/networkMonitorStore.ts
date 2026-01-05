import { create } from 'zustand';
import { useShallow } from 'zustand/react/shallow';

// =============================================================================
// Types
// =============================================================================

export interface NetworkRequest {
  id: string;
  cmd: string;
  args?: Record<string, unknown>;
  startTime: number;
  endTime?: number;
  duration?: number;
  status: 'pending' | 'success' | 'error';
  response?: unknown;
  error?: unknown;
}

// =============================================================================
// State Types (Separated from Actions for SOTA 2026 patterns)
// =============================================================================

interface NetworkMonitorStateSlice {
  requests: NetworkRequest[];
  isOpen: boolean;
  isRecording: boolean;
}

interface NetworkMonitorActionsSlice {
  addRequest: (request: NetworkRequest) => void;
  updateRequest: (id: string, updates: Partial<NetworkRequest>) => void;
  clearRequests: () => void;
  setIsOpen: (isOpen: boolean) => void;
  toggleRecording: () => void;
}

type NetworkMonitorState = NetworkMonitorStateSlice & NetworkMonitorActionsSlice;

// =============================================================================
// Store Implementation
// =============================================================================

export const useNetworkMonitorStore = create<NetworkMonitorState>((set) => ({
  // Initial State
  requests: [],
  isOpen: false,
  isRecording: true,

  // Actions
  addRequest: (request) => {
    set((state) => {
      if (!state.isRecording) return state;
      return { requests: [request, ...state.requests].slice(0, 1000) }; // Keep last 1000 requests
    });
  },

  updateRequest: (id, updates) => {
    set((state) => ({
      requests: state.requests.map((req) =>
        req.id === id ? { ...req, ...updates } : req
      ),
    }));
  },

  clearRequests: () => {
    set({ requests: [] });
  },

  setIsOpen: (isOpen) => {
    set({ isOpen });
  },

  toggleRecording: () => {
    set((state) => ({ isRecording: !state.isRecording }));
  },
}));

// =============================================================================
// Atomic Selectors (SOTA 2026: Prevents unnecessary re-renders)
// =============================================================================

/** Select requests array - only re-renders when requests change */
export const useRequests = () => useNetworkMonitorStore(state => state.requests);

/** Select isOpen state - only re-renders when isOpen changes */
export const useIsMonitorOpen = () => useNetworkMonitorStore(state => state.isOpen);

/** Select isRecording state - only re-renders when isRecording changes */
export const useIsRecording = () => useNetworkMonitorStore(state => state.isRecording);

// =============================================================================
// Derived Selectors
// =============================================================================

/** Select request count */
export const useRequestCount = () => useNetworkMonitorStore(state => state.requests.length);

/** Select pending requests count */
export const usePendingRequestCount = () => useNetworkMonitorStore(
  state => state.requests.filter(r => r.status === 'pending').length
);

/** Select error requests count */
export const useErrorRequestCount = () => useNetworkMonitorStore(
  state => state.requests.filter(r => r.status === 'error').length
);

/** Select a specific request by id */
export const useRequestById = (id: string) => useNetworkMonitorStore(
  state => state.requests.find(r => r.id === id)
);

// =============================================================================
// Composite Selectors with useShallow
// =============================================================================

/** Select monitor UI state together */
export const useMonitorUIState = () => useNetworkMonitorStore(
  useShallow(state => ({
    isOpen: state.isOpen,
    isRecording: state.isRecording,
  }))
);

/** Select request statistics */
export const useRequestStats = () => useNetworkMonitorStore(
  useShallow(state => {
    const total = state.requests.length;
    const pending = state.requests.filter(r => r.status === 'pending').length;
    const success = state.requests.filter(r => r.status === 'success').length;
    const error = state.requests.filter(r => r.status === 'error').length;
    return { total, pending, success, error };
  })
);

// =============================================================================
// Action Selectors (Stable references - never cause re-renders)
// =============================================================================

/** Get all network monitor actions - stable reference */
export const useNetworkMonitorActions = () => useNetworkMonitorStore(
  useShallow(state => ({
    addRequest: state.addRequest,
    updateRequest: state.updateRequest,
    clearRequests: state.clearRequests,
    setIsOpen: state.setIsOpen,
    toggleRecording: state.toggleRecording,
  }))
);
