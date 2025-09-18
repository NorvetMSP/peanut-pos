import React from 'react';

type SearchBarProps = {
  query: string;
  onQueryChange: (value: string) => void;
};

const SearchBar: React.FC<SearchBarProps> = ({ query, onQueryChange }) => (
  <div className="flex-1">
    <label className="block text-sm font-medium text-gray-600 mb-1" htmlFor="pos-product-search">
      Search products
    </label>
    <input
      id="pos-product-search"
      value={query}
      onChange={event => onQueryChange(event.target.value)}
      placeholder="Search products..."
      className="w-full px-3 py-2 border border-gray-300 rounded focus:outline-none focus:ring-1 focus:ring-cyan-500"
      type="search"
    />
  </div>
);

export default SearchBar;
