import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import StatsCard from './StatsCard';
import { Users, Zap, Clock, AlertCircle } from 'lucide-react';

describe('StatsCard', () => {
  describe('rendering', () => {
    it('renders the title', () => {
      render(<StatsCard icon={Users} title="Total Users" value={100} />);
      expect(screen.getByText('Total Users')).toBeInTheDocument();
    });

    it('renders the value as a number', () => {
      render(<StatsCard icon={Users} title="Total Users" value={100} />);
      expect(screen.getByText('100')).toBeInTheDocument();
    });

    it('renders the value as a string', () => {
      render(<StatsCard icon={Zap} title="Status" value="Active" />);
      expect(screen.getByText('Active')).toBeInTheDocument();
    });

    it('renders the description when provided', () => {
      render(
        <StatsCard
          icon={Clock}
          title="Uptime"
          value="99.9%"
          description="Last 30 days"
        />
      );
      expect(screen.getByText('Last 30 days')).toBeInTheDocument();
    });

    it('does not render description when not provided', () => {
      render(<StatsCard icon={Users} title="Total Users" value={100} />);
      // Check there's no stat-desc element with content
      const statDesc = document.querySelector('.stat-desc');
      expect(statDesc).toBeNull();
    });
  });

  describe('color classes', () => {
    it('uses primary color class by default', () => {
      const { container } = render(
        <StatsCard icon={Users} title="Users" value={50} />
      );
      const statFigure = container.querySelector('.stat-figure');
      expect(statFigure).toHaveClass('text-primary');

      const statValue = container.querySelector('.stat-value');
      expect(statValue).toHaveClass('text-primary');
    });

    it('applies custom color class', () => {
      const { container } = render(
        <StatsCard icon={AlertCircle} title="Errors" value={5} colorClass="error" />
      );
      const statFigure = container.querySelector('.stat-figure');
      expect(statFigure).toHaveClass('text-error');

      const statValue = container.querySelector('.stat-value');
      expect(statValue).toHaveClass('text-error');
    });

    it('applies success color class', () => {
      const { container } = render(
        <StatsCard icon={Zap} title="Active" value="Yes" colorClass="success" />
      );
      const statFigure = container.querySelector('.stat-figure');
      expect(statFigure).toHaveClass('text-success');
    });
  });

  describe('icon rendering', () => {
    it('renders the provided icon', () => {
      render(<StatsCard icon={Users} title="Users" value={50} />);
      // The icon should be rendered inside stat-figure
      const statFigure = document.querySelector('.stat-figure');
      expect(statFigure).toBeInTheDocument();
      expect(statFigure?.querySelector('svg')).toBeInTheDocument();
    });

    it('icon has correct size class', () => {
      render(<StatsCard icon={Zap} title="Power" value="On" />);
      const svg = document.querySelector('.stat-figure svg');
      expect(svg).toHaveClass('w-8', 'h-8');
    });
  });

  describe('structure', () => {
    it('has stat container with correct classes', () => {
      const { container } = render(
        <StatsCard icon={Users} title="Users" value={50} />
      );
      const stat = container.querySelector('.stat');
      expect(stat).toHaveClass('bg-base-100', 'shadow', 'rounded-lg');
    });

    it('contains stat-title element', () => {
      render(<StatsCard icon={Users} title="My Title" value={50} />);
      const statTitle = document.querySelector('.stat-title');
      expect(statTitle).toBeInTheDocument();
      expect(statTitle).toHaveTextContent('My Title');
    });

    it('contains stat-value element', () => {
      render(<StatsCard icon={Users} title="Users" value={123} />);
      const statValue = document.querySelector('.stat-value');
      expect(statValue).toBeInTheDocument();
      expect(statValue).toHaveTextContent('123');
    });
  });

  describe('different value types', () => {
    it('handles zero value', () => {
      render(<StatsCard icon={Users} title="Users" value={0} />);
      expect(screen.getByText('0')).toBeInTheDocument();
    });

    it('handles large numbers', () => {
      render(<StatsCard icon={Users} title="Users" value={1000000} />);
      expect(screen.getByText('1000000')).toBeInTheDocument();
    });

    it('handles decimal numbers', () => {
      render(<StatsCard icon={Zap} title="Rate" value={99.99} />);
      expect(screen.getByText('99.99')).toBeInTheDocument();
    });

    it('handles empty string value', () => {
      render(<StatsCard icon={Users} title="Status" value="" />);
      const statValue = document.querySelector('.stat-value');
      expect(statValue).toBeInTheDocument();
    });

    it('handles special characters in value', () => {
      render(<StatsCard icon={Zap} title="Progress" value="50% complete" />);
      expect(screen.getByText('50% complete')).toBeInTheDocument();
    });
  });
});
