Ext.define('PrefixField', {
    extend: 'Ext.form.field.Text',
    alias: 'widget.prefixfield',

    maxLength: 6,
    allowBlank: false,

    maskRe: /([A-Za-z]+)$/,

    listeners: {
	change: function(field) {
	    field.setValue(field.getValue().toUpperCase());
	},
    },
});
