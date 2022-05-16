Ext.define('PBS.form.VerifyOutdatedAfter', {
    extend: 'Proxmox.form.field.Integer',
    alias: 'widget.pbsVerifyOutdatedAfter',

    name: 'outdated-after',

    minValue: 1,
    value: 30,
    allowBlank: false,

    triggers: {
	clear: {
	    cls: 'pmx-clear-trigger',
	    weight: -1,
	    hidden: false,
	    handler: function() {
		this.triggers.clear.setVisible(false);
		this.setValue('');
	    },
	},
    },

    listeners: {
	change: function(field, value) {
	    let canClear = value !== '';
	    field.triggers.clear.setVisible(canClear);
	},
    },
});

