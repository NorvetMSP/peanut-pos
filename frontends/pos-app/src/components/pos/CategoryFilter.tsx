import React from 'react';

type CategoryFilterProps = {
  categories: string[];
  selected: string;
  onSelect: (category: string) => void;
};

const CategoryFilter: React.FC<CategoryFilterProps> = ({ categories, selected, onSelect }) => (
  <div className="min-w-[180px]">
    <label className="block text-sm font-medium text-gray-600 mb-1" htmlFor="pos-category-filter">
      Category
    </label>
    <select
      id="pos-category-filter"
      value={selected}
      onChange={event => onSelect(event.target.value)}
      className="w-full px-3 py-2 border border-gray-300 rounded focus:outline-none focus:ring-1 focus:ring-cyan-500"
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
