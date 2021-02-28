const paper_sizes = {
    a4: {
	comment: 'A4 (plain)',
	page_width: 210,
	page_height: 297,
    },
    letter: {
	comment: 'Letter (plain)',
	page_width: 215.9,
	page_height: 279.4,
    },
    avery3420: {
	fixed: true,
	comment: 'Avery Zweckform 3420',
	page_width: 210,
	page_height: 297,
	label_width: 70,
	label_height: 16.9,
	margin_left: 0,
	margin_top: 5,
	column_spacing: 0,
	row_spacing: 0,
    },
};

function paper_size_combo_data() {
    let data = [];

    for (let [key, value] of Object.entries(paper_sizes)) {
	data.push({ value: key, text: value.comment });
    }
    return data;
}

Ext.define('PaperSize', {
    extend: 'Ext.form.field.ComboBox',
    alias: 'widget.paperSize',

    editable: false,

    displayField: 'text',
    valueField: 'value',
    queryMode: 'local',

    store: {
	field: ['value', 'text'],
	data: paper_size_combo_data(),
    },
});
