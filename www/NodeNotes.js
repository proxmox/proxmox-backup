// Needs to be its own xtype for `path` to work in `NavigationTree`
Ext.define('PBS.NodeNotes', {
    extend: 'Ext.panel.Panel',
    xtype: 'pbsNodeNotes',

    scrollable: true,
    layout: 'fit',

    items: [
	{
	    xtype: 'container',
	    layout: 'fit',
	    items: [{
		xtype: 'pmxNotesView',
		tools: false,
		border: false,
		node: 'localhost',
		enableTBar: true,
		maxLength: 1022*64,
	    }],
	},
    ],
});
