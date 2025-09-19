import React from 'react';

type CategoryFilterProps = {
  categories: string[];
  selected: string;
  onSelect: (category: string) => void;
};

const CategoryFilter: React.FC<CategoryFilterProps> = ({ categories, selected, onSelect }) => (
  <div className="cashier-field">
    <label className="cashier-field__label" htmlFor="pos-category-filter">
      Category
    </label>
    <select
      id="pos-category-filter"
      value={selected}
      onChange={event => onSelect(event.target.value)}
      className="cashier-select"
    >
      {categories.map(category => (
        <option key={category} value={category}>
          {category}
        </option>
      ))}
    </select>
  </div>
);

export default CategoryFilter;
