import React from 'react';

type SearchBarProps = {
  query: string;
  onQueryChange: (value: string) => void;
};

const SearchBar: React.FC<SearchBarProps> = ({ query, onQueryChange }) => (
  <div className="cashier-field">
    <label className="cashier-field__label" htmlFor="pos-product-search">
      Search products
    </label>
    <input
      id="pos-product-search"
      value={query}
      onChange={event => onQueryChange(event.target.value)}
      placeholder="Search products..."
      className="cashier-input"
      type="search"
    />
  </div>
);

export default SearchBar;
