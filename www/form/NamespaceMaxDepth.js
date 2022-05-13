Ext.define('PBS.form.NamespaceMaxDepth', {
    extend: 'Proxmox.form.field.Integer',
    alias: 'widget.pbsNamespaceMaxDepth',

    allowBlank: true,

    emptyText: gettext('Full'),
    fieldLabel: gettext('Max. Depth'),
    deleteEmpty: true,

    minValue: 0,
    maxValue: 7,

    triggers: {
	clear: {
	    cls: 'pmx-clear-trigger',
	    weight: -1,
	    hidden: true,
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

Ext.define('PBS.form.NamespaceMaxDepthReduced', {
    extend: 'PBS.form.NamespaceMaxDepth',
    alias: 'widget.pbsNamespaceMaxDepthReduced',

    calcMaxPrefixLength: function(ns1, ns2) {
	let maxPrefixLength = 0;
	if (ns1 !== undefined && ns1 !== null) {
	    maxPrefixLength = (ns1.match(/[/]/g) || []).length + (ns1 === '' ? 0 : 1);
	}
	if (ns2 !== undefined && ns2 !== null) {
	    let ns2PrefixLength = (ns2.match(/[/]/g) || []).length + (ns2 === '' ? 0 : 1);
	    if (ns2PrefixLength > maxPrefixLength) {
		maxPrefixLength = ns2PrefixLength;
	    }
	}
	return maxPrefixLength;
    },

    setLimit: function(ns1, ns2) {
	let me = this;
	let maxPrefixLength = me.calcMaxPrefixLength(ns1, ns2);
	if (maxPrefixLength !== undefined) {
	    me.maxValue = 7 - maxPrefixLength;
	} else {
	    me.maxValue = 7;
	}
    },
});
