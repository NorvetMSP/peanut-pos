import React from 'react';

type SearchBarProps = {
  query: string;
  onQueryChange: (value: string) => void;
  onSubmitQuery?: (value: string) => void; // optional: triggered on Enter key
};

const SearchBar: React.FC<SearchBarProps> = ({ query, onQueryChange, onSubmitQuery }) => (
  <div className="cashier-field">
    <label className="cashier-field__label" htmlFor="pos-product-search">
      Search products
    </label>
    <input
      id="pos-product-search"
      value={query}
      onChange={event => onQueryChange(event.target.value)}
      onKeyDown={event => {
        if (event.key === 'Enter' && typeof onSubmitQuery === 'function') {
          onSubmitQuery(query);
        }
      }}
      placeholder="Search products..."
      className="cashier-input"
      type="search"
    />
  </div>
);

export default SearchBar;
