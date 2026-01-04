import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import Pagination from './Pagination';

// Mock react-i18next
vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, params?: Record<string, unknown>) => {
      const translations: Record<string, string> = {
        'common.prev_page': 'Previous',
        'common.next_page': 'Next',
        'common.per_page': 'Per page',
        'common.items': 'items',
      };

      if (key === 'common.pagination_info' && params) {
        return `Showing ${params.start} to ${params.end} of ${params.total} results`;
      }

      return translations[key] || key;
    },
  }),
}));

describe('Pagination', () => {
  const defaultProps = {
    currentPage: 1,
    totalPages: 10,
    onPageChange: vi.fn(),
    totalItems: 100,
    itemsPerPage: 10,
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe('rendering', () => {
    it('renders pagination info', () => {
      render(<Pagination {...defaultProps} />);
      expect(screen.getByText('Showing 1 to 10 of 100 results')).toBeInTheDocument();
    });

    it('returns null when totalPages is 1 and no page size change callback', () => {
      const { container } = render(
        <Pagination {...defaultProps} totalPages={1} />
      );
      expect(container.firstChild).toBeNull();
    });

    it('renders when totalPages is 1 but onPageSizeChange is provided', () => {
      render(
        <Pagination {...defaultProps} totalPages={1} onPageSizeChange={vi.fn()} />
      );
      expect(screen.getByText('Showing 1 to 10 of 100 results')).toBeInTheDocument();
    });

    it('renders page buttons', () => {
      render(<Pagination {...defaultProps} />);
      // Should show pages 1-5 when on page 1
      expect(screen.getByRole('button', { name: '1' })).toBeInTheDocument();
      expect(screen.getByRole('button', { name: '2' })).toBeInTheDocument();
    });
  });

  describe('navigation', () => {
    it('calls onPageChange when clicking a page number', async () => {
      const user = userEvent.setup();
      const onPageChange = vi.fn();
      render(<Pagination {...defaultProps} onPageChange={onPageChange} />);

      await user.click(screen.getByRole('button', { name: '3' }));
      expect(onPageChange).toHaveBeenCalledWith(3);
    });

    it('calls onPageChange when clicking next button', async () => {
      const user = userEvent.setup();
      const onPageChange = vi.fn();
      render(<Pagination {...defaultProps} onPageChange={onPageChange} />);

      // Find the next button (has sr-only text "Next")
      const nextButtons = screen.getAllByRole('button');
      const nextButton = nextButtons.find(btn =>
        btn.querySelector('.sr-only')?.textContent === 'Next'
      );

      if (nextButton) {
        await user.click(nextButton);
        expect(onPageChange).toHaveBeenCalledWith(2);
      }
    });

    it('calls onPageChange when clicking previous button', async () => {
      const user = userEvent.setup();
      const onPageChange = vi.fn();
      render(
        <Pagination {...defaultProps} currentPage={5} onPageChange={onPageChange} />
      );

      const prevButtons = screen.getAllByRole('button');
      const prevButton = prevButtons.find(btn =>
        btn.querySelector('.sr-only')?.textContent === 'Previous'
      );

      if (prevButton) {
        await user.click(prevButton);
        expect(onPageChange).toHaveBeenCalledWith(4);
      }
    });

    it('disables previous button on first page', () => {
      render(<Pagination {...defaultProps} currentPage={1} />);

      const buttons = screen.getAllByRole('button');
      const prevButton = buttons.find(btn =>
        btn.querySelector('.sr-only')?.textContent === 'Previous'
      );

      expect(prevButton).toBeDisabled();
    });

    it('disables next button on last page', () => {
      render(<Pagination {...defaultProps} currentPage={10} />);

      const buttons = screen.getAllByRole('button');
      const nextButton = buttons.find(btn =>
        btn.querySelector('.sr-only')?.textContent === 'Next'
      );

      expect(nextButton).toBeDisabled();
    });
  });

  describe('current page indicator', () => {
    it('marks the current page with aria-current', () => {
      render(<Pagination {...defaultProps} currentPage={3} />);
      const currentPageButton = screen.getByRole('button', { name: '3' });
      expect(currentPageButton).toHaveAttribute('aria-current', 'page');
    });

    it('applies active styles to current page', () => {
      render(<Pagination {...defaultProps} currentPage={3} />);
      const currentPageButton = screen.getByRole('button', { name: '3' });
      expect(currentPageButton).toHaveClass('bg-blue-600');
    });
  });

  describe('page range calculation', () => {
    it('shows ellipsis when there are many pages', () => {
      render(<Pagination {...defaultProps} currentPage={5} totalPages={20} />);
      // Should show ellipsis for skipped pages
      const ellipses = screen.getAllByText('...');
      expect(ellipses.length).toBeGreaterThan(0);
    });

    it('shows first page link when not in first pages', () => {
      render(<Pagination {...defaultProps} currentPage={10} totalPages={20} />);
      expect(screen.getByRole('button', { name: '1' })).toBeInTheDocument();
    });

    it('shows last page link when not in last pages', () => {
      render(<Pagination {...defaultProps} currentPage={5} totalPages={20} />);
      expect(screen.getByRole('button', { name: '20' })).toBeInTheDocument();
    });
  });

  describe('page size selector', () => {
    it('renders page size selector when onPageSizeChange is provided', () => {
      render(
        <Pagination {...defaultProps} onPageSizeChange={vi.fn()} />
      );
      expect(screen.getByRole('combobox')).toBeInTheDocument();
    });

    it('does not render page size selector when onPageSizeChange is not provided', () => {
      render(<Pagination {...defaultProps} />);
      expect(screen.queryByRole('combobox')).not.toBeInTheDocument();
    });

    it('calls onPageSizeChange when selecting a new page size', async () => {
      const user = userEvent.setup();
      const onPageSizeChange = vi.fn();
      render(
        <Pagination {...defaultProps} onPageSizeChange={onPageSizeChange} />
      );

      const select = screen.getByRole('combobox');
      await user.selectOptions(select, '20');
      expect(onPageSizeChange).toHaveBeenCalledWith(20);
    });

    it('renders custom page size options', () => {
      render(
        <Pagination
          {...defaultProps}
          onPageSizeChange={vi.fn()}
          pageSizeOptions={[5, 15, 25]}
        />
      );

      const select = screen.getByRole('combobox');
      expect(select).toContainHTML('5');
      expect(select).toContainHTML('15');
      expect(select).toContainHTML('25');
    });
  });

  describe('pagination info calculation', () => {
    it('calculates correct range for middle pages', () => {
      render(
        <Pagination
          {...defaultProps}
          currentPage={5}
          itemsPerPage={10}
          totalItems={100}
        />
      );
      expect(screen.getByText('Showing 41 to 50 of 100 results')).toBeInTheDocument();
    });

    it('calculates correct range for last page with partial items', () => {
      render(
        <Pagination
          {...defaultProps}
          currentPage={4}
          totalPages={4}
          itemsPerPage={10}
          totalItems={35}
        />
      );
      expect(screen.getByText('Showing 31 to 35 of 35 results')).toBeInTheDocument();
    });
  });
});
